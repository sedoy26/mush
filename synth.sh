#!/usr/bin/env bash
# ─────────────────────────────────────────
#  MurSynth — terminal synthesizer + drums
# ─────────────────────────────────────────

VENV_DIR="$HOME/.mursynth-venv"
PY_SCRIPT="$HOME/.mursynth.py"

cat > "$PY_SCRIPT" << 'PYEOF'
import curses, numpy as np, sounddevice as sd
import os, shutil, subprocess, threading, time, textwrap, wave
from collections import deque

try:
    import mido
    MIDI_AVAILABLE = True
    MIDI_IMPORT_ERROR = ""
except Exception as exc:
    mido = None
    MIDI_AVAILABLE = False
    MIDI_IMPORT_ERROR = str(exc)

SAMPLE_RATE   = 44100
BLOCK_SIZE    = 1024
SCOPE_SAMPLES = 2048
MAX_VOICES    = 6
MAX_LOOP_SECONDS = 8.0
DECLICK_TIME  = 0.004
DRUM_ATTACK_TIME = 0.002
DRUM_RETRIG_DECLICK_TIME = 0.003
DRUM_END_FADE_TIME = 0.005
MASTER_HEADROOM = 0.72
LIMIT_CEILING = 0.92
OUTPUT_GAIN = 0.5
DC_BLOCK_R = 0.995
MOD_SMOOTH_TIME = 0.02
PARAM_SMOOTH_TIME = 0.015
FILTER_SMOOTH_TIME = 0.012
FX_SMOOTH_TIME = 0.02
LOOP_WRAP_FADE_TIME = 0.008
LOOP_PLAY_SMOOTH_TIME = 0.012
MASTER_LIMIT_THRESHOLD = 0.74
MASTER_LIMIT_ATTACK = 0.002
MASTER_LIMIT_RELEASE = 0.08
LOOP_UNDO_LIMIT = 12
GLOBAL_REC_FILENAME = "untiteled.wav"
WAV_DIRNAME = "wav"
CAMERA_CAPTURE_FPS = 30
CAMERA_OUTPUT_FPS = 12
CAMERA_FRAME_WIDTH = 160
CAMERA_FRAME_HEIGHT = 90
CAMERA_DEVICE = os.environ.get("MURSYNTH_CAMERA_DEVICE", "0")
INPUT_POLL_MS = 16
ESC_KEY_DELAY_MS = 25

KEYBOARD_OFFSETS = {
    ord('a'):0,  ord('w'):1,  ord('s'):2,  ord('e'):3,
    ord('d'):4,  ord('f'):5,  ord('t'):6,  ord('g'):7,
    ord('y'):8,  ord('h'):9,  ord('u'):10, ord('j'):11,
    ord('k'):12,
}
NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]
THEME_NAMES = ["MAGENTA", "MINT", "AMBER"]
VISUAL_MODES = ["SCOPE", "CAM"]
CAMERA_REACTIVE_STYLES = [
    "OFF",
    "KICK FLASH",
    "SYNTH GLOW",
    "DRUM PUNCH",
    "BASS SCAN",
    "EDGE PULSE",
    "GLITCH SHIFT",
    "GATE POSTER",
    "FIRE STORM",
    "ICE PULSE",
    "CHROMA SPLIT",
    "MATRIX BEAT",
]
SETTINGS_PAGE_NAMES = ["MAIN", "CAM FX", "SOUND DEVICE", "MIDI DEVICE", "MIDI NOTE", "MIDI MAP"]
MIDI_PAD_TARGETS = [
    ("pad_kick", "Pad Kick", "note"),
    ("pad_snare", "Pad Snare", "note"),
    ("pad_clap", "Pad Clap", "note"),
    ("pad_hihat", "Pad HiHat", "note"),
    ("pad_tom", "Pad Tom", "note"),
    ("pad_cym", "Pad Cym", "note"),
    ("pad_loop_play", "Pad Loop Play", "note"),
    ("pad_global_rec", "Pad Wav Rec", "note"),
]
MIDI_CC_TARGETS = [
    ("cc_volume", "Knob Volume", "cc"),
    ("cc_cutoff", "Knob Cutoff", "cc"),
    ("cc_resonance", "Knob Reson", "cc"),
    ("cc_drive", "Knob Drive", "cc"),
    ("cc_delay_mix", "Knob Delay Mix", "cc"),
    ("cc_delay_feedback", "Knob Delay Fbk", "cc"),
    ("cc_delay_time", "Knob Delay Time", "cc"),
    ("cc_warmth", "Knob Warmth", "cc"),
    ("cc_air", "Knob Air", "cc"),
    ("cc_reverb", "Knob Reverb", "cc"),
    ("cc_bpm", "Knob Drum BPM", "cc"),
    ("cc_osc1_level", "Knob Osc1 Level", "cc"),
    ("cc_osc2_level", "Knob Osc2 Level", "cc"),
]
MIDI_BIND_TARGETS = MIDI_PAD_TARGETS + MIDI_CC_TARGETS
MIDI_BIND_LABELS = {target_id: label for target_id, label, _ in MIDI_BIND_TARGETS}
MIDI_BIND_KINDS = {target_id: kind for target_id, _, kind in MIDI_BIND_TARGETS}

# ═══════════════════════════════════════════════════════════════════════════════
#  SYNTH STATE
# ═══════════════════════════════════════════════════════════════════════════════
synth = {
    "volume": 0.5,
    "volume_current": 0.5,
    "base_midi": 60, "key_offset": None, "midi_note": None,
    "attack":  0.01, "release": 0.3, "env": 0.0,
    "gate_mode": 0, "key_note_on": False, "midi_note_on": False,
    "lfo_wave": 0, "lfo_rate": 2.0, "lfo_depth": 0.0,
    "lfo_depth_current": 0.0,
    "lfo_target": 0, "lfo_phase": 0.0,
    "filter_on": False, "cutoff": 0.8, "resonance": 0.0,
    "oscillators": [
        {
            "waveform": 0,
            "level": 0.75,
            "octave": 0,
            "detune_cents": 0.0,
            "voice_phases": [0.0] * MAX_VOICES,
        },
        {
            "waveform": 1,
            "level": 0.45,
            "octave": -1,
            "detune_cents": -3.0,
            "voice_phases": [0.0] * MAX_VOICES,
        },
    ],
    "active_osc": 0,
    "voices": 1,
    "freq_current": 261.63,
    "fx_drive": 0.0,
    "fx_delay_mix": 0.0,
    "fx_delay_mix_current": 0.0,
    "fx_delay_feedback": 0.20,
    "fx_delay_feedback_current": 0.20,
    "fx_delay_time": 0.25,
    "fx_delay_time_current": 0.25,
    "fx_drive_current": 0.0,
    "fx_warmth": 0.0,
    "fx_warmth_current": 0.0,
    "fx_air": 0.0,
    "fx_air_current": 0.0,
    "fx_reverb": 0.0,
    "fx_reverb_current": 0.0,
    "filter_mix": 0.0,
    "filt_z": 0.0, "filt_z2": 0.0,
    "last_output": 0.0,
    "xruns": 0,
}
synth_lock = threading.Lock()

loop = {
    "buffer": np.zeros(int(SAMPLE_RATE * MAX_LOOP_SECONDS), dtype=np.float32),
    "length": 0,
    "write_pos": 0,
    "read_pos": 0,
    "recording": False,
    "playing": False,
    "overdub": False,
    "has_audio": False,
    "play_gain": 0.0,
    "undo_stack": [],
}
loop_lock = threading.Lock()

global_rec = {
    "recording": False,
    "chunks": [],
    "last_path": "",
    "last_error": "",
}
global_rec_lock = threading.Lock()

ui_state = {
    "theme": 0,
    "visual_mode": 0,
    "camera_style": 0,
    "camera_reactivity": 0.85,
    "scope_show_drums": True,
}
ui_lock = threading.Lock()
camera_state = {
    "gray": None,
    "error": "Camera idle",
    "thread": None,
    "process": None,
    "stop_event": threading.Event(),
    "running": False,
}
camera_lock = threading.Lock()
reactive_state = {"master": 0.0, "synth": 0.0, "drums": 0.0, "kick": 0.0, "snare": 0.0, "hat": 0.0, "note": 0.0}
reactive_lock = threading.Lock()

audio = {
    "inputs": [],
    "outputs": [],
    "input_index": 0,
    "output_index": 0,
    "input_name": "",
    "output_name": "",
    "status": "Audio idle",
}
audio_lock = threading.Lock()

midi = {
    "enabled": False,
    "devices": [],
    "device_index": 0,
    "device_name": "",
    "channel": -1,
    "note_input": True,
    "pad_input": True,
    "status": "MIDI unavailable" if not MIDI_AVAILABLE else "MIDI idle",
    "last_message": "",
    "input_port": None,
    "learn_mode": "off",
    "learn_target_index": 0,
    "note_edit_in": 60,
    "note_edit_out": 60,
    "note_map": {},
    "note_bindings": {target_id: None for target_id, _, kind in MIDI_BIND_TARGETS if kind == "note"},
    "cc_bindings": {target_id: None for target_id, _, kind in MIDI_BIND_TARGETS if kind == "cc"},
    "held_notes": {},
    "held_order": [],
}
midi_lock = threading.Lock()

# ═══════════════════════════════════════════════════════════════════════════════
#  DRUM STATE
# ═══════════════════════════════════════════════════════════════════════════════
DRUM_NAMES  = ["KICK", "SNRE", "CLAP", "HIHT", "TOM ", "CYMB"]
DRUM_COLORS = [3, 4, 5, 6]   # colour pair indices per voice
NUM_DRUM_VOICES = len(DRUM_NAMES)
NUM_STEPS   = 32

DRUM_BANKS = [
    {
        "name": "TR-808",
        "lengths": [int(0.62 * SAMPLE_RATE), int(0.20 * SAMPLE_RATE), int(0.07 * SAMPLE_RATE), int(0.38 * SAMPLE_RATE)],
        "kick": {"base": 38.0, "sweep": 122.0, "pitch_decay": 0.09, "amp_decay": 0.48, "click": 0.05, "click_decay": 0.004},
        "snare": {"tone1": 185.0, "tone2": 330.0, "tone_mix": 0.34, "noise_mix": 0.66, "tone_decay": 0.16, "noise_decay": 0.11, "hp_freq": 1900.0},
        "hihat": {"decay": 0.045, "hp_freq": 9000.0, "noise_mix": 0.72, "metal_mix": 0.28, "metal_freqs": [4020.0, 5220.0, 6460.0, 8120.0, 9300.0, 10500.0]},
        "top": {"decay": 0.30, "hp_freq": 6400.0, "noise_mix": 0.78, "metal_mix": 0.22, "metal_freqs": [3180.0, 4140.0, 5300.0, 6680.0, 7940.0, 9460.0]},
    },
    {
        "name": "TR-909",
        "lengths": [int(0.46 * SAMPLE_RATE), int(0.16 * SAMPLE_RATE), int(0.06 * SAMPLE_RATE), int(0.24 * SAMPLE_RATE)],
        "kick": {"base": 52.0, "sweep": 146.0, "pitch_decay": 0.06, "amp_decay": 0.34, "click": 0.11, "click_decay": 0.003},
        "snare": {"tone1": 228.0, "tone2": 342.0, "tone_mix": 0.42, "noise_mix": 0.58, "tone_decay": 0.12, "noise_decay": 0.09, "hp_freq": 2500.0},
        "hihat": {"decay": 0.035, "hp_freq": 9800.0, "noise_mix": 0.48, "metal_mix": 0.52, "metal_freqs": [4180.0, 5480.0, 6420.0, 8360.0, 9340.0, 11020.0]},
        "top": {"decay": 0.18, "hp_freq": 7200.0, "noise_mix": 0.44, "metal_mix": 0.56, "metal_freqs": [3320.0, 4280.0, 5840.0, 7140.0, 8620.0, 10300.0]},
    },
    {
        "name": "CR-78",
        "lengths": [int(0.36 * SAMPLE_RATE), int(0.15 * SAMPLE_RATE), int(0.05 * SAMPLE_RATE), int(0.22 * SAMPLE_RATE)],
        "kick": {"base": 64.0, "sweep": 82.0, "pitch_decay": 0.05, "amp_decay": 0.22, "click": 0.03, "click_decay": 0.003},
        "snare": {"tone1": 240.0, "tone2": 480.0, "tone_mix": 0.52, "noise_mix": 0.48, "tone_decay": 0.10, "noise_decay": 0.08, "hp_freq": 1800.0},
        "hihat": {"decay": 0.028, "hp_freq": 8200.0, "noise_mix": 0.84, "metal_mix": 0.16, "metal_freqs": [3640.0, 4980.0, 6420.0, 7980.0, 9100.0, 10080.0]},
        "top": {"decay": 0.14, "hp_freq": 5800.0, "noise_mix": 0.86, "metal_mix": 0.14, "metal_freqs": [3020.0, 3940.0, 4820.0, 6020.0, 7360.0, 8920.0]},
    },
    {
        "name": "LinnDrum",
        "lengths": [int(0.34 * SAMPLE_RATE), int(0.16 * SAMPLE_RATE), int(0.06 * SAMPLE_RATE), int(0.18 * SAMPLE_RATE)],
        "kick": {"base": 58.0, "sweep": 96.0, "pitch_decay": 0.045, "amp_decay": 0.24, "click": 0.08, "click_decay": 0.003},
        "snare": {"tone1": 198.0, "tone2": 286.0, "tone_mix": 0.46, "noise_mix": 0.54, "tone_decay": 0.11, "noise_decay": 0.09, "hp_freq": 2200.0},
        "hihat": {"decay": 0.038, "hp_freq": 9400.0, "noise_mix": 0.76, "metal_mix": 0.24, "metal_freqs": [4280.0, 5260.0, 6120.0, 7360.0, 8640.0, 9560.0]},
        "top": {"decay": 0.15, "hp_freq": 6800.0, "noise_mix": 0.74, "metal_mix": 0.26, "metal_freqs": [3160.0, 4180.0, 5120.0, 6340.0, 7820.0, 9140.0]},
    },
    {
        "name": "DMX",
        "lengths": [int(0.38 * SAMPLE_RATE), int(0.17 * SAMPLE_RATE), int(0.06 * SAMPLE_RATE), int(0.18 * SAMPLE_RATE)],
        "kick": {"base": 49.0, "sweep": 110.0, "pitch_decay": 0.05, "amp_decay": 0.28, "click": 0.09, "click_decay": 0.003},
        "snare": {"tone1": 210.0, "tone2": 300.0, "tone_mix": 0.40, "noise_mix": 0.60, "tone_decay": 0.11, "noise_decay": 0.09, "hp_freq": 2400.0},
        "hihat": {"decay": 0.03, "hp_freq": 9800.0, "noise_mix": 0.79, "metal_mix": 0.21, "metal_freqs": [4520.0, 5600.0, 6480.0, 7740.0, 8980.0, 10140.0]},
        "top": {"decay": 0.16, "hp_freq": 7000.0, "noise_mix": 0.78, "metal_mix": 0.22, "metal_freqs": [3340.0, 4260.0, 5480.0, 6740.0, 8120.0, 9440.0]},
    },
    {
        "name": "DrumTraks",
        "lengths": [int(0.36 * SAMPLE_RATE), int(0.16 * SAMPLE_RATE), int(0.06 * SAMPLE_RATE), int(0.16 * SAMPLE_RATE)],
        "kick": {"base": 54.0, "sweep": 92.0, "pitch_decay": 0.05, "amp_decay": 0.26, "click": 0.07, "click_decay": 0.003},
        "snare": {"tone1": 224.0, "tone2": 312.0, "tone_mix": 0.38, "noise_mix": 0.62, "tone_decay": 0.10, "noise_decay": 0.08, "hp_freq": 2600.0},
        "hihat": {"decay": 0.03, "hp_freq": 10100.0, "noise_mix": 0.70, "metal_mix": 0.30, "metal_freqs": [4340.0, 5440.0, 6300.0, 7540.0, 8860.0, 10340.0]},
        "top": {"decay": 0.14, "hp_freq": 7200.0, "noise_mix": 0.68, "metal_mix": 0.32, "metal_freqs": [3240.0, 4320.0, 5480.0, 6740.0, 8260.0, 9680.0]},
    },
    {
        "name": "Simmons",
        "lengths": [int(0.44 * SAMPLE_RATE), int(0.20 * SAMPLE_RATE), int(0.08 * SAMPLE_RATE), int(0.22 * SAMPLE_RATE)],
        "kick": {"base": 46.0, "sweep": 168.0, "pitch_decay": 0.08, "amp_decay": 0.30, "click": 0.02, "click_decay": 0.003},
        "snare": {"tone1": 278.0, "tone2": 418.0, "tone_mix": 0.66, "noise_mix": 0.34, "tone_decay": 0.14, "noise_decay": 0.08, "hp_freq": 2800.0},
        "hihat": {"decay": 0.05, "hp_freq": 8800.0, "noise_mix": 0.34, "metal_mix": 0.66, "metal_freqs": [2820.0, 3560.0, 4720.0, 6040.0, 7440.0, 8920.0]},
        "top": {"decay": 0.22, "hp_freq": 6400.0, "noise_mix": 0.30, "metal_mix": 0.70, "metal_freqs": [2340.0, 3140.0, 4260.0, 5480.0, 6920.0, 8400.0]},
    },
    {
        "name": "RX11",
        "lengths": [int(0.32 * SAMPLE_RATE), int(0.15 * SAMPLE_RATE), int(0.05 * SAMPLE_RATE), int(0.15 * SAMPLE_RATE)],
        "kick": {"base": 60.0, "sweep": 88.0, "pitch_decay": 0.04, "amp_decay": 0.22, "click": 0.08, "click_decay": 0.0025},
        "snare": {"tone1": 236.0, "tone2": 330.0, "tone_mix": 0.36, "noise_mix": 0.64, "tone_decay": 0.09, "noise_decay": 0.07, "hp_freq": 2900.0},
        "hihat": {"decay": 0.025, "hp_freq": 10400.0, "noise_mix": 0.74, "metal_mix": 0.26, "metal_freqs": [4680.0, 5720.0, 6640.0, 7860.0, 9180.0, 10560.0]},
        "top": {"decay": 0.12, "hp_freq": 7600.0, "noise_mix": 0.70, "metal_mix": 0.30, "metal_freqs": [3460.0, 4460.0, 5640.0, 6920.0, 8420.0, 9840.0]},
    },
    {
        "name": "R-8",
        "lengths": [int(0.40 * SAMPLE_RATE), int(0.18 * SAMPLE_RATE), int(0.07 * SAMPLE_RATE), int(0.24 * SAMPLE_RATE)],
        "kick": {"base": 50.0, "sweep": 118.0, "pitch_decay": 0.055, "amp_decay": 0.28, "click": 0.07, "click_decay": 0.003},
        "snare": {"tone1": 212.0, "tone2": 324.0, "tone_mix": 0.44, "noise_mix": 0.56, "tone_decay": 0.11, "noise_decay": 0.08, "hp_freq": 2600.0},
        "hihat": {"decay": 0.036, "hp_freq": 9800.0, "noise_mix": 0.62, "metal_mix": 0.38, "metal_freqs": [4180.0, 5160.0, 6280.0, 7580.0, 8980.0, 10440.0]},
        "top": {"decay": 0.18, "hp_freq": 7200.0, "noise_mix": 0.60, "metal_mix": 0.40, "metal_freqs": [3260.0, 4320.0, 5520.0, 6840.0, 8340.0, 9880.0]},
    },
    {
        "name": "SP12",
        "lengths": [int(0.42 * SAMPLE_RATE), int(0.18 * SAMPLE_RATE), int(0.06 * SAMPLE_RATE), int(0.20 * SAMPLE_RATE)],
        "kick": {"base": 48.0, "sweep": 104.0, "pitch_decay": 0.05, "amp_decay": 0.30, "click": 0.06, "click_decay": 0.003},
        "snare": {"tone1": 204.0, "tone2": 296.0, "tone_mix": 0.40, "noise_mix": 0.60, "tone_decay": 0.11, "noise_decay": 0.09, "hp_freq": 2300.0},
        "hihat": {"decay": 0.032, "hp_freq": 9400.0, "noise_mix": 0.80, "metal_mix": 0.20, "metal_freqs": [4120.0, 5180.0, 6200.0, 7420.0, 8640.0, 9780.0]},
        "top": {"decay": 0.17, "hp_freq": 6900.0, "noise_mix": 0.78, "metal_mix": 0.22, "metal_freqs": [3180.0, 4220.0, 5340.0, 6620.0, 8040.0, 9480.0]},
    },
]

def enrich_drum_bank(bank):
    kick = bank["kick"]
    snare = bank["snare"]
    hihat = bank["hihat"]
    cymb = bank["top"]
    clap = {
        "bursts": [0.0, 0.011, 0.023],
        "burst_decay": max(0.015, snare["noise_decay"] * 0.45),
        "decay": max(0.05, snare["noise_decay"] * 0.8),
        "hp_freq": snare["hp_freq"] * 1.35,
        "noise_mix": min(0.9, snare["noise_mix"] + 0.12),
        "tone_mix": max(0.08, snare["tone_mix"] * 0.45),
        "tone_freq": snare["tone2"] * 1.15,
    }
    tom = {
        "base": kick["base"] * 2.4,
        "sweep": kick["sweep"] * 0.45,
        "pitch_decay": max(0.04, kick["pitch_decay"] * 1.15),
        "amp_decay": max(0.11, kick["amp_decay"] * 0.52),
        "tone2": snare["tone1"] * 0.8,
        "tone_mix": 0.3,
    }
    bank["clap"] = clap
    bank["tom"] = tom
    bank["cymb"] = cymb
    bank["lengths"] = [
        bank["lengths"][0],
        bank["lengths"][1],
        int(max(0.08, snare["noise_decay"] * 1.4) * SAMPLE_RATE),
        bank["lengths"][2],
        int(max(0.14, kick["amp_decay"] * 0.6) * SAMPLE_RATE),
        bank["lengths"][3],
    ]
    return bank

DRUM_BANKS = [enrich_drum_bank(bank) for bank in DRUM_BANKS]

drum = {
    "steps":    [[False]*NUM_STEPS for _ in range(NUM_DRUM_VOICES)],   # [voice][step]
    "vol":      [0.8, 0.7, 0.65, 0.6, 0.62, 0.65],
    "running":  False,
    "bpm":      120.0,
    "bank":     0,
    "cur_step": 0,
    "trig":     [False]*NUM_DRUM_VOICES,   # one-shot trigger flags set by sequencer thread
}
drum_lock = threading.Lock()

scope_buf  = deque([0.0]*SCOPE_SAMPLES, maxlen=SCOPE_SAMPLES)
scope_lock = threading.Lock()

WAVEFORMS   = ["SINE","SAW ","SQR ","TRI "]
LFO_WAVES   = ["SIN","TRI","SQR"]
LFO_TARGETS = ["PTCH","VOL ","FILT"]

def clamp(v,lo,hi): return max(lo,min(hi,v))

def midi_to_freq(midi):
    return 440.0 * (2.0 ** ((midi - 69) / 12.0))

def midi_to_name(midi):
    midi = int(midi)
    return f"{NOTE_NAMES[midi % 12]}{midi // 12 - 1}"

def sync_pitch_locked():
    target = midi_to_freq(current_play_midi_locked())
    if synth["freq_current"] <= 0.0:
        synth["freq_current"] = target

def current_play_midi_locked():
    if synth["midi_note_on"] and synth["midi_note"] is not None:
        return int(synth["midi_note"])
    active = synth["key_offset"]
    return int(synth["base_midi"]) + (active if active is not None else 0)

def note_active_locked():
    return bool(synth["key_note_on"] or synth["midi_note_on"])

def current_play_freq_locked():
    return midi_to_freq(current_play_midi_locked())

def active_osc_locked():
    return synth["oscillators"][synth["active_osc"]]

def current_drum_bank_locked():
    return DRUM_BANKS[drum["bank"]]

def draw_visual_line(scr, y, x, line, default_attr=0):
    if isinstance(line, str):
        safe_addstr(scr, y, x, line, default_attr)
        return
    cx = x
    for segment, attr in line:
        if not segment:
            continue
        safe_addstr(scr, y, cx, segment, attr)
        cx += len(segment)

def short_label(text, width):
    text = str(text)
    if len(text) <= width:
        return text
    return text[:max(0, width - 1)] + "…"

def format_binding_value(kind, value):
    if value is None:
        return "--"
    return f"CC {value:03d}" if kind == "cc" else f"NOTE {midi_to_name(value):>3}"

def format_midi_message(message):
    if hasattr(message, "channel"):
        ch_text = f" ch{message.channel + 1}"
    else:
        ch_text = ""
    if message.type == "note_on" and getattr(message, "velocity", 0) > 0:
        return f"NOTE {midi_to_name(message.note):>3} v{message.velocity:03d}{ch_text}"
    if message.type in ("note_off", "note_on"):
        return f"NOTE OFF {midi_to_name(message.note):>3}{ch_text}"
    if message.type == "control_change":
        return f"CC {message.control:03d} val{message.value:03d}{ch_text}"
    return short_label(str(message), 40)

def refresh_midi_devices_locked():
    if not MIDI_AVAILABLE:
        midi["devices"] = []
        midi["device_index"] = 0
        midi["device_name"] = ""
        midi["status"] = short_label(f"MIDI unavailable: {MIDI_IMPORT_ERROR}", 42)
        return
    try:
        names = list(mido.get_input_names())
    except Exception as exc:
        midi["devices"] = []
        midi["device_index"] = 0
        midi["device_name"] = ""
        midi["status"] = short_label(f"MIDI scan failed: {exc}", 42)
        return

    current_name = midi["device_name"]
    midi["devices"] = names
    if not names:
        midi["device_index"] = 0
        midi["device_name"] = ""
        if midi["enabled"]:
            midi["status"] = "No MIDI inputs found"
        return
    if current_name in names:
        midi["device_index"] = names.index(current_name)
        midi["device_name"] = current_name
    else:
        midi["device_index"] = int(clamp(midi["device_index"], 0, len(names) - 1))
        midi["device_name"] = names[midi["device_index"]]

def set_midi_note_locked(note, is_on):
    if is_on:
        synth["midi_note"] = int(note)
        synth["midi_note_on"] = True
    else:
        synth["midi_note_on"] = False
        synth["midi_note"] = None
    sync_pitch_locked()

def clear_midi_note_state():
    with midi_lock:
        midi["held_notes"].clear()
        midi["held_order"] = []
    with synth_lock:
        set_midi_note_locked(None, False)

def handle_pad_action(target_id):
    if target_id == "pad_kick":
        with drum_lock: drum["trig"][0] = True
    elif target_id == "pad_snare":
        with drum_lock: drum["trig"][1] = True
    elif target_id == "pad_clap":
        with drum_lock: drum["trig"][2] = True
    elif target_id == "pad_hihat":
        with drum_lock: drum["trig"][3] = True
    elif target_id == "pad_tom":
        with drum_lock: drum["trig"][4] = True
    elif target_id == "pad_cym":
        with drum_lock: drum["trig"][5] = True
    elif target_id == "pad_loop_play":
        with loop_lock:
            if loop["has_audio"]:
                loop["playing"] = not loop["playing"]
    elif target_id == "pad_global_rec":
        if global_rec["recording"]:
            stop_global_recording()
        else:
            start_global_recording()

def apply_midi_cc_target(target_id, value):
    norm = clamp(value / 127.0, 0.0, 1.0)
    if target_id == "cc_bpm":
        with drum_lock:
            drum["bpm"] = 40.0 + norm * 260.0
        return

    with synth_lock:
        if target_id == "cc_volume":
            synth["volume"] = norm
        elif target_id == "cc_cutoff":
            synth["filter_on"] = True
            synth["cutoff"] = norm
        elif target_id == "cc_resonance":
            synth["filter_on"] = True
            synth["resonance"] = norm * 0.99
        elif target_id == "cc_drive":
            synth["fx_drive"] = norm
        elif target_id == "cc_delay_mix":
            synth["fx_delay_mix"] = norm
        elif target_id == "cc_delay_feedback":
            synth["fx_delay_feedback"] = norm * 0.95
        elif target_id == "cc_delay_time":
            synth["fx_delay_time"] = norm
        elif target_id == "cc_warmth":
            synth["fx_warmth"] = norm
        elif target_id == "cc_air":
            synth["fx_air"] = norm
        elif target_id == "cc_reverb":
            synth["fx_reverb"] = norm
        elif target_id == "cc_osc1_level":
            synth["oscillators"][0]["level"] = norm
        elif target_id == "cc_osc2_level":
            synth["oscillators"][1]["level"] = norm

def on_midi_message(message):
    if hasattr(message, "channel"):
        msg_channel = int(message.channel)
    else:
        msg_channel = -1

    with midi_lock:
        midi["last_message"] = format_midi_message(message)
        learn_mode = midi["learn_mode"]
        learn_target = MIDI_BIND_TARGETS[midi["learn_target_index"]]
        enabled = midi["enabled"]
        selected_channel = midi["channel"]
        note_input = midi["note_input"]
        pad_input = midi["pad_input"]

        if learn_mode == "note_src" and message.type == "note_on" and getattr(message, "velocity", 0) > 0:
            midi["note_edit_in"] = int(message.note)
            midi["learn_mode"] = "off"
            midi["status"] = f"Source note learned: {midi_to_name(message.note)}"
            return

        if learn_mode == "bind":
            target_id, label, kind = learn_target
            if kind == "cc" and message.type == "control_change":
                midi["cc_bindings"][target_id] = int(message.control)
                midi["learn_mode"] = "off"
                midi["status"] = f"Bound {label} to CC {message.control}"
                return
            if kind == "note" and message.type == "note_on" and getattr(message, "velocity", 0) > 0:
                midi["note_bindings"][target_id] = int(message.note)
                midi["learn_mode"] = "off"
                midi["status"] = f"Bound {label} to {midi_to_name(message.note)}"
                return

        if not enabled:
            return
        if selected_channel >= 0 and msg_channel >= 0 and msg_channel != selected_channel:
            return

        pad_target = None
        if pad_input and message.type == "note_on" and getattr(message, "velocity", 0) > 0:
            for target_id, note_value in midi["note_bindings"].items():
                if note_value == int(message.note):
                    pad_target = target_id
                    break

        mapped_note = midi["note_map"].get(int(getattr(message, "note", 0)), int(getattr(message, "note", 0)))
        if pad_target is None and message.type == "note_on" and getattr(message, "velocity", 0) > 0 and note_input:
            midi["held_notes"][int(message.note)] = mapped_note
            midi["held_order"] = [src for src in midi["held_order"] if src != int(message.note)] + [int(message.note)]
        elif pad_target is None and message.type in ("note_off", "note_on") and (message.type == "note_off" or getattr(message, "velocity", 0) == 0):
            midi["held_notes"].pop(int(message.note), None)
            midi["held_order"] = [src for src in midi["held_order"] if src != int(message.note)]

    if pad_target is not None:
        handle_pad_action(pad_target)
        return

    if message.type == "control_change":
        with midi_lock:
            cc_target = next((target_id for target_id, cc_value in midi["cc_bindings"].items() if cc_value == int(message.control)), None)
        if cc_target is not None:
            apply_midi_cc_target(cc_target, int(message.value))
        return

    if note_input and message.type in ("note_on", "note_off"):
        with midi_lock:
            active_note = midi["held_notes"][midi["held_order"][-1]] if midi["held_order"] else None
        with synth_lock:
            if active_note is None:
                set_midi_note_locked(None, False)
            else:
                set_midi_note_locked(active_note, True)

def reopen_midi_input():
    with midi_lock:
        refresh_midi_devices_locked()
        enabled = midi["enabled"] and MIDI_AVAILABLE
        device_name = midi["device_name"]
        old_port = midi["input_port"]
        midi["input_port"] = None

    if old_port is not None:
        try:
            old_port.close()
        except Exception:
            pass

    if not enabled:
        clear_midi_note_state()
        with midi_lock:
            midi["status"] = "MIDI disabled" if MIDI_AVAILABLE else short_label(f"MIDI unavailable: {MIDI_IMPORT_ERROR}", 42)
        return

    if not device_name:
        with midi_lock:
            midi["status"] = "No MIDI input selected"
        return

    try:
        port = mido.open_input(device_name, callback=on_midi_message)
        with midi_lock:
            midi["input_port"] = port
            midi["status"] = short_label(f"Listening: {device_name}", 42)
    except Exception as exc:
        with midi_lock:
            midi["status"] = short_label(f"MIDI open failed: {exc}", 42)

noise_rng = np.random.default_rng()

# ═══════════════════════════════════════════════════════════════════════════════
#  DRUM SYNTHESIS  (simple 808/909 style)
# ═══════════════════════════════════════════════════════════════════════════════
# Per-voice state for synthesis (running phase/filter per voice)
drum_voice = [
    {"phase": 0.0, "fz": 0.0, "fz2": 0.0, "t": 0, "last_output": 0.0, "declick_from": 0.0, "declick_pos": 0, "aux_phases": [0.0]*6}
    for _ in range(NUM_DRUM_VOICES)
]
drum_trig_sample = [-1]*NUM_DRUM_VOICES   # sample index when each voice was triggered (-1=off)

DRUM_ATTACK_SAMPLES = max(1, int(DRUM_ATTACK_TIME * SAMPLE_RATE))
DRUM_RETRIG_DECLICK_SAMPLES = max(1, int(DRUM_RETRIG_DECLICK_TIME * SAMPLE_RATE))
DRUM_END_FADE_SAMPLES = max(1, int(DRUM_END_FADE_TIME * SAMPLE_RATE))

def reset_drum_voice(voice_idx):
    dv = drum_voice[voice_idx]
    dv["declick_from"] = dv["last_output"]
    dv["declick_pos"] = 0
    dv["phase"] = 0.0
    dv["fz"] = 0.0
    dv["fz2"] = 0.0
    dv["t"] = 0
    dv["aux_phases"] = [0.0] * len(dv["aux_phases"])

def metallic_cluster(dv, freqs, frames):
    if frames <= 0 or not freqs:
        return np.zeros(frames, dtype=np.float32)
    out = np.zeros(frames, dtype=np.float32)
    phases = dv["aux_phases"]
    count = min(len(phases), len(freqs))
    time_steps = np.arange(frames, dtype=np.float32) + 1.0
    for idx in range(count):
        phase = phases[idx] + time_steps * (float(freqs[idx]) / SAMPLE_RATE)
        phases[idx] = float(phase[-1] % 1.0)
        out += np.where(np.remainder(phase, 1.0) < 0.5, 1.0, -1.0).astype(np.float32, copy=False)
    out /= max(1, count)
    return out

def apply_drum_attack(signal, t_samp):
    remaining = DRUM_ATTACK_SAMPLES - int(t_samp)
    if remaining <= 0 or len(signal) == 0:
        return signal
    fade = min(len(signal), remaining)
    ramp = (np.arange(fade, dtype=np.float32) + float(t_samp)) / float(DRUM_ATTACK_SAMPLES)
    signal[:fade] *= np.clip(ramp, 0.0, 1.0)
    return signal

def finalize_drum_chunk(signal, voice_idx, t_samp, length_samples):
    out = apply_drum_attack(signal.astype(np.float32, copy=False), t_samp)
    dv = drum_voice[voice_idx]
    remaining = DRUM_RETRIG_DECLICK_SAMPLES - dv["declick_pos"]
    if remaining > 0 and len(out) > 0:
        fade = min(len(out), remaining)
        ramp = (np.arange(fade, dtype=np.float32) + dv["declick_pos"] + 1.0) / float(DRUM_RETRIG_DECLICK_SAMPLES)
        out[:fade] = dv["declick_from"] * (1.0 - ramp) + out[:fade] * ramp
        dv["declick_pos"] += fade
        if dv["declick_pos"] >= DRUM_RETRIG_DECLICK_SAMPLES:
            dv["declick_from"] = 0.0
    if len(out) > 0:
        samples_left = length_samples - (t_samp + np.arange(len(out), dtype=np.float32) + 1.0)
        tail_gain = np.clip(samples_left / float(DRUM_END_FADE_SAMPLES), 0.0, 1.0)
        out *= tail_gain
    if len(out) > 0:
        dv["last_output"] = float(out[-1])
    return out

def drum_synth_kick(bank, t_samp, frames):
    if frames <= 0:
        return np.zeros(0, dtype=np.float32)
    params = bank["kick"]
    dv  = drum_voice[0]
    age = t_samp + np.arange(frames, dtype=np.float32)
    freq = params["base"] + params["sweep"] * np.exp(-age / max(1.0, params["pitch_decay"] * SAMPLE_RATE))
    amp = np.exp(-age / max(1.0, params["amp_decay"] * SAMPLE_RATE))
    phase = dv["phase"] + np.cumsum(freq / SAMPLE_RATE, dtype=np.float32)
    dv["phase"] = float(phase[-1] % 1.0)
    body = np.sin(2*np.pi*phase)
    click_env = np.exp(-age / max(1.0, params["click_decay"] * SAMPLE_RATE))
    click = noise_rng.uniform(-1.0, 1.0, frames).astype(np.float32) * click_env
    out = (body * (1.0 - params["click"]) + click * params["click"]) * amp
    return finalize_drum_chunk(out, 0, t_samp, bank["lengths"][0])

def drum_synth_snare(bank, t_samp, frames):
    if frames <= 0:
        return np.zeros(0, dtype=np.float32)
    params = bank["snare"]
    dv  = drum_voice[1]
    age = t_samp + np.arange(frames, dtype=np.float32)
    time_steps = np.arange(frames, dtype=np.float32) + 1.0
    phase1 = dv["phase"] + time_steps * (params["tone1"] / SAMPLE_RATE)
    phase2 = dv["aux_phases"][0] + time_steps * (params["tone2"] / SAMPLE_RATE)
    dv["phase"] = float(phase1[-1] % 1.0)
    dv["aux_phases"][0] = float(phase2[-1] % 1.0)
    tone_env = np.exp(-age / max(1.0, params["tone_decay"] * SAMPLE_RATE))
    noise_env = np.exp(-age / max(1.0, params["noise_decay"] * SAMPLE_RATE))
    tone = (np.sin(2*np.pi*phase1) * 0.65 + np.sin(2*np.pi*phase2) * 0.35) * tone_env
    hp_coeff = np.exp(-2*np.pi * params["hp_freq"] / SAMPLE_RATE)
    noise = noise_rng.uniform(-1.0, 1.0, frames).astype(np.float32)
    hp_noise = np.empty(frames, dtype=np.float32)
    for i, sample in enumerate(noise):
        dv["fz"] = dv["fz"] * hp_coeff + sample * (1.0 - hp_coeff)
        hp_noise[i] = sample - dv["fz"]
    out = tone * params["tone_mix"] + hp_noise * noise_env * params["noise_mix"]
    return finalize_drum_chunk(out, 1, t_samp, bank["lengths"][1])

def drum_synth_hihat(bank, t_samp, frames):
    out = np.zeros(frames, dtype=np.float32)
    params = bank["hihat"]
    dv  = drum_voice[3]
    c   = np.exp(-2*np.pi * params["hp_freq"] / SAMPLE_RATE)
    noise = noise_rng.uniform(-1.0, 1.0, frames).astype(np.float32)
    metal = metallic_cluster(dv, params["metal_freqs"], frames)
    for i in range(frames):
        age   = t_samp + i
        amp   = np.exp(-age / max(1.0, params["decay"] * SAMPLE_RATE))
        sample = noise[i]
        dv["fz"] = dv["fz"] * c + sample * (1.0-c)
        out[i] = ((sample - dv["fz"]) * params["noise_mix"] + metal[i] * params["metal_mix"]) * amp
    return finalize_drum_chunk(out, 3, t_samp, bank["lengths"][3])

def drum_synth_clap(bank, t_samp, frames):
    out = np.zeros(frames, dtype=np.float32)
    params = bank["clap"]
    dv  = drum_voice[2]
    c   = np.exp(-2*np.pi * params["hp_freq"] / SAMPLE_RATE)
    noise = noise_rng.uniform(-1.0, 1.0, frames).astype(np.float32)
    phase = dv["phase"] + (np.arange(frames, dtype=np.float32) + 1.0) * (params["tone_freq"] / SAMPLE_RATE)
    dv["phase"] = float(phase[-1] % 1.0) if frames > 0 else dv["phase"]
    tone = np.sin(2.0 * np.pi * phase).astype(np.float32, copy=False)
    for i in range(frames):
        age   = t_samp + i
        sample = noise[i]
        dv["fz"] = dv["fz"] * c + sample * (1.0-c)
        burst_env = 0.0
        for burst in params["bursts"]:
            if age >= burst * SAMPLE_RATE:
                burst_env += np.exp(-(age - burst * SAMPLE_RATE) / max(1.0, params["burst_decay"] * SAMPLE_RATE))
        amp = burst_env * np.exp(-age / max(1.0, params["decay"] * SAMPLE_RATE))
        out[i] = ((sample - dv["fz"]) * params["noise_mix"] + tone[i] * params["tone_mix"]) * amp
    return finalize_drum_chunk(out, 2, t_samp, bank["lengths"][2])

def drum_synth_tom(bank, t_samp, frames):
    if frames <= 0:
        return np.zeros(0, dtype=np.float32)
    params = bank["tom"]
    dv = drum_voice[4]
    age = t_samp + np.arange(frames, dtype=np.float32)
    freq = params["base"] + params["sweep"] * np.exp(-age / max(1.0, params["pitch_decay"] * SAMPLE_RATE))
    amp = np.exp(-age / max(1.0, params["amp_decay"] * SAMPLE_RATE))
    phase1 = dv["phase"] + np.cumsum(freq / SAMPLE_RATE, dtype=np.float32)
    phase2 = dv["aux_phases"][0] + np.cumsum((freq * 1.5) / SAMPLE_RATE, dtype=np.float32)
    dv["phase"] = float(phase1[-1] % 1.0)
    dv["aux_phases"][0] = float(phase2[-1] % 1.0)
    body = np.sin(2*np.pi*phase1) * (1.0 - params["tone_mix"]) + np.sin(2*np.pi*phase2) * params["tone_mix"]
    out = body * amp
    return finalize_drum_chunk(out, 4, t_samp, bank["lengths"][4])

def drum_synth_cymb(bank, t_samp, frames):
    out = np.zeros(frames, dtype=np.float32)
    params = bank["cymb"]
    dv  = drum_voice[5]
    c   = np.exp(-2*np.pi * params["hp_freq"] / SAMPLE_RATE)
    noise = noise_rng.uniform(-1.0, 1.0, frames).astype(np.float32)
    metal = metallic_cluster(dv, params["metal_freqs"], frames)
    for i in range(frames):
        age   = t_samp + i
        amp   = np.exp(-age / max(1.0, params["decay"] * SAMPLE_RATE))
        sample = noise[i]
        dv["fz"] = dv["fz"] * c + sample * (1.0-c)
        out[i] = ((sample - dv["fz"]) * params["noise_mix"] + metal[i] * params["metal_mix"]) * amp
    return finalize_drum_chunk(out, 5, t_samp, bank["lengths"][5])

DRUM_SYNTHS  = [drum_synth_kick, drum_synth_snare, drum_synth_clap, drum_synth_hihat, drum_synth_tom, drum_synth_cymb]

# ═══════════════════════════════════════════════════════════════════════════════
#  SYNTH DSP
# ═══════════════════════════════════════════════════════════════════════════════
def osc_sample(phi, wf):
    if   wf==0: return float(np.sin(phi))
    elif wf==1: return 2*(phi/(2*np.pi)%1)-1
    elif wf==2: return float(np.sign(np.sin(phi)))
    else:       return 2*abs(2*(phi/(2*np.pi)%1)-1)-1

def poly_blep(phase, dt):
    dt = np.maximum(dt, 1e-6).astype(np.float32, copy=False)
    out = np.zeros_like(phase, dtype=np.float32)

    rising = phase < dt
    if np.any(rising):
        t = phase[rising] / dt[rising]
        out[rising] = t + t - t * t - 1.0

    falling = phase > (1.0 - dt)
    if np.any(falling):
        t = (phase[falling] - 1.0) / dt[falling]
        out[falling] = t * t + t + t + 1.0

    return out

def osc_block(phases, wf, phase_steps=None):
    phase = np.remainder(phases, 1.0).astype(np.float32, copy=False)
    if wf == 0:
        return np.sin(phase * (2.0 * np.pi)).astype(np.float32, copy=False)
    if wf == 1:
        if phase_steps is None:
            return (phase * 2.0 - 1.0).astype(np.float32, copy=False)
        dt = np.minimum(np.abs(phase_steps).astype(np.float32, copy=False), 0.5)
        return ((phase * 2.0 - 1.0) - poly_blep(phase, dt)).astype(np.float32, copy=False)
    if wf == 2:
        if phase_steps is None:
            return np.where(phase < 0.5, 1.0, -1.0).astype(np.float32, copy=False)
        dt = np.minimum(np.abs(phase_steps).astype(np.float32, copy=False), 0.5)
        square = np.where(phase < 0.5, 1.0, -1.0).astype(np.float32, copy=False)
        square += poly_blep(phase, dt)
        square -= poly_blep(np.remainder(phase + 0.5, 1.0).astype(np.float32, copy=False), dt)
        return square.astype(np.float32, copy=False)
    return (2.0 * np.abs(2.0 * phase - 1.0) - 1.0).astype(np.float32, copy=False)

def cutoff_coeff(cn, res):
    w = 2*np.pi * (20.0 * (20000.0/20.0)**cn) / SAMPLE_RATE
    return np.exp(-w), res

def soft_limit(signal, drive=1.35, ceiling=LIMIT_CEILING):
    drive = max(1.0, float(drive))
    ceiling = max(1e-6, float(ceiling))
    threshold = ceiling / drive
    out = np.array(signal, dtype=np.float32, copy=True)
    abs_out = np.abs(out)
    over = abs_out > threshold
    if np.any(over):
        headroom = max(ceiling - threshold, 1e-6)
        shaped = threshold + np.tanh((abs_out[over] - threshold) / headroom) * headroom
        out[over] = np.sign(out[over]) * np.minimum(shaped, ceiling)
    return out

def soft_saturate_sample(sample, drive=1.1, ceiling=LIMIT_CEILING):
    drive = max(1.0, float(drive))
    ceiling = max(1e-6, float(ceiling))
    threshold = ceiling / drive
    sample = float(sample)
    mag = abs(sample)
    if mag <= threshold:
        return sample
    headroom = max(ceiling - threshold, 1e-6)
    shaped = threshold + np.tanh((mag - threshold) / headroom) * headroom
    return float(np.sign(sample) * min(shaped, ceiling))

def apply_master_limiter(signal, state):
    if len(signal) == 0:
        return signal
    gain = float(state["gain"])
    out = np.empty_like(signal)
    future_peak = np.maximum.accumulate(np.abs(signal[::-1]))[::-1]
    allowed_gain = np.minimum(1.0, MASTER_LIMIT_THRESHOLD / np.maximum(future_peak, 1e-6))
    release = 1.0 - np.exp(-1.0 / max(1.0, MASTER_LIMIT_RELEASE * SAMPLE_RATE))
    gain = min(gain, float(allowed_gain[0]))
    for i, sample in enumerate(signal):
        target_gain = float(allowed_gain[i])
        if target_gain < gain:
            gain = target_gain
        else:
            gain += (target_gain - gain) * release
            gain = min(gain, target_gain)
        out[i] = sample * gain
    state["gain"] = float(gain)
    return out

def dc_block(signal, state):
    x1 = state["x1"]
    y1 = state["y1"]
    out = np.empty_like(signal)
    for i, x0 in enumerate(signal):
        y0 = x0 - x1 + DC_BLOCK_R * y1
        out[i] = y0
        x1 = x0
        y1 = y0
    state["x1"] = float(x1)
    state["y1"] = float(y1)
    return out

def finalize_loop_edges_locked():
    length = loop["length"]
    if length < 8:
        return
    fade = min(int(0.002 * SAMPLE_RATE), length)
    if fade <= 0:
        return
    ramp = np.linspace(0.0, 1.0, fade, dtype=np.float32)
    loop["buffer"][:fade] *= ramp

def clear_loop_locked():
    loop["buffer"].fill(0.0)
    loop["length"] = 0
    loop["write_pos"] = 0
    loop["read_pos"] = 0
    loop["recording"] = False
    loop["playing"] = False
    loop["overdub"] = False
    loop["has_audio"] = False
    loop["play_gain"] = 0.0
    loop["undo_stack"].clear()

def push_loop_undo_locked():
    if not loop["has_audio"] or loop["length"] <= 0:
        return
    loop["undo_stack"].append({
        "buffer": loop["buffer"][:loop["length"]].copy(),
        "length": int(loop["length"]),
    })
    if len(loop["undo_stack"]) > LOOP_UNDO_LIMIT:
        loop["undo_stack"] = loop["undo_stack"][-LOOP_UNDO_LIMIT:]

def undo_last_overdub():
    with loop_lock:
        if loop["recording"] or not loop["undo_stack"]:
            return False
        snap = loop["undo_stack"].pop()
        undo_stack = list(loop["undo_stack"])
        clear_loop_locked()
        loop["undo_stack"] = undo_stack
        loop["length"] = snap["length"]
        if loop["length"] > 0:
            loop["buffer"][:loop["length"]] = snap["buffer"]
            loop["has_audio"] = True
            loop["playing"] = True
        return True

def start_global_recording():
    with global_rec_lock:
        global_rec["recording"] = True
        global_rec["chunks"] = []
        global_rec["last_error"] = ""
        global_rec["last_path"] = ""

def next_global_recording_path():
    base, ext = os.path.splitext(GLOBAL_REC_FILENAME)
    wav_dir = os.path.join(os.getcwd(), WAV_DIRNAME)
    os.makedirs(wav_dir, exist_ok=True)
    path = os.path.join(wav_dir, GLOBAL_REC_FILENAME)
    if not os.path.exists(path):
        return path

    idx = 1
    while True:
        candidate = os.path.join(wav_dir, f"{base}-{idx:04d}{ext}")
        if not os.path.exists(candidate):
            return candidate
        idx += 1

def stop_global_recording():
    with global_rec_lock:
        if not global_rec["recording"]:
            return global_rec["last_path"], global_rec["last_error"]
        global_rec["recording"] = False
        chunks = global_rec["chunks"]
        global_rec["chunks"] = []
    if not chunks:
        with global_rec_lock:
            global_rec["last_error"] = "No audio captured"
        return "", "No audio captured"
    try:
        path = next_global_recording_path()
        mono = np.concatenate(chunks).astype(np.float32, copy=False)
        pcm = np.clip(mono, -0.999, 0.999)
        pcm = (pcm * 32767.0).astype(np.int16)
        stereo = np.repeat(pcm[:, None], 2, axis=1)
        with wave.open(path, "wb") as wav_file:
            wav_file.setnchannels(2)
            wav_file.setsampwidth(2)
            wav_file.setframerate(SAMPLE_RATE)
            wav_file.writeframes(stereo.tobytes())
        with global_rec_lock:
            global_rec["last_path"] = path
            global_rec["last_error"] = ""
        return path, ""
    except Exception as exc:
        with global_rec_lock:
            global_rec["last_error"] = str(exc)
        return "", str(exc)

def stop_loop_recording():
    with loop_lock:
        if not loop["recording"]:
            return
        loop["recording"] = False
        loop["length"] = min(loop["write_pos"], len(loop["buffer"]))
        loop["has_audio"] = loop["length"] > int(0.08 * SAMPLE_RATE)
        loop["playing"] = loop["has_audio"]
        loop["overdub"] = False
        loop["read_pos"] = 0
        if loop["has_audio"]:
            finalize_loop_edges_locked()

def process_loop(live_signal):
    frames = len(live_signal)
    loop_out = np.zeros(frames, dtype=np.float32)
    with loop_lock:
        has_audio = loop["has_audio"] and loop["length"] > 0
        length = loop["length"]
        playing = loop["playing"] and has_audio
        recording = loop["recording"]
        overdub = loop["overdub"] and has_audio
        play_gain = float(loop["play_gain"])
        play_target = 1.0 if playing else 0.0
        play_alpha = 1.0 - np.exp(-1.0 / max(1.0, LOOP_PLAY_SMOOTH_TIME * SAMPLE_RATE))
        wrap_fade = 0
        if has_audio and length > 1:
            wrap_fade = min(max(1, int(LOOP_WRAP_FADE_TIME * SAMPLE_RATE)), max(1, length // 4))
        for i in range(frames):
            read_pos = loop["read_pos"]
            write_pos = loop["write_pos"]
            existing = loop["buffer"][read_pos] if has_audio else 0.0
            if playing and wrap_fade > 1 and read_pos >= length - wrap_fade:
                wrap_idx = read_pos - (length - wrap_fade)
                wrap_mix = (wrap_idx + 1.0) / float(wrap_fade)
                existing = existing * (1.0 - wrap_mix) + loop["buffer"][wrap_idx] * wrap_mix

            play_gain += (play_target - play_gain) * play_alpha
            loop_out[i] = existing * play_gain

            if recording:
                if overdub:
                    layered = existing * 0.72 + live_signal[i] * 0.45
                    loop["buffer"][write_pos] = soft_saturate_sample(layered, drive=1.02, ceiling=0.9)
                else:
                    loop["buffer"][write_pos] = soft_saturate_sample(live_signal[i] * 0.9, drive=1.01, ceiling=0.9)
                loop["write_pos"] += 1
                if not has_audio:
                    loop["length"] = max(loop["length"], loop["write_pos"])
                if loop["write_pos"] >= len(loop["buffer"]):
                    loop["write_pos"] = len(loop["buffer"])
                    finalize_loop_edges_locked()
                    loop["recording"] = False
                    loop["overdub"] = False
                    loop["length"] = len(loop["buffer"])
                    loop["has_audio"] = True
                    loop["playing"] = True
                    loop["read_pos"] = 0
                    has_audio = True
                    length = loop["length"]
                    playing = True
                    break

            if playing:
                loop["read_pos"] = 0 if read_pos + 1 >= length else read_pos + 1
            if recording and overdub:
                loop["write_pos"] = 0 if write_pos + 1 >= length else write_pos + 1

        loop["play_gain"] = float(play_gain)

    return loop_out

def gen_synth(frames):
    with synth_lock:
        target_freq = current_play_freq_locked(); vol_target = synth["volume"]
        env       = synth["env"];   note   = note_active_locked()
        gate      = synth["gate_mode"]
        atk       = max(synth["attack"],  0.001)
        rel       = max(synth["release"], 0.001)
        lfo_wf    = synth["lfo_wave"];  lfo_rate  = synth["lfo_rate"]
        lfo_dep_target = synth["lfo_depth"]; lfo_dep = synth["lfo_depth_current"]; lfo_tgt   = synth["lfo_target"]
        lfo_ph    = synth["lfo_phase"]
        filt_on   = synth["filter_on"]; cutoff    = synth["cutoff"]
        filter_mix = synth["filter_mix"]
        res       = synth["resonance"]
        voices    = int(clamp(synth["voices"], 1, MAX_VOICES))
        oscillators = [
            {
                "waveform": osc["waveform"],
                "level": osc["level"],
                "octave": osc["octave"],
                "detune_cents": osc["detune_cents"],
                "voice_phases": osc["voice_phases"][:],
            }
            for osc in synth["oscillators"]
        ]
        fz        = synth["filt_z"];    fz2       = synth["filt_z2"]
        freq_cur  = synth["freq_current"]
        vol_cur   = synth["volume_current"]
        last_out  = synth["last_output"]

    gate_open = (gate==1) or note
    out = np.zeros(frames, dtype=np.float32)

    if not gate_open and env < 0.0001:
        with synth_lock:
            synth["env"]=0.0; synth["filt_z"]=0.0; synth["filt_z2"]=0.0
            synth["lfo_phase"] = (lfo_ph + lfo_rate*frames/SAMPLE_RATE) % 1.0
            synth["last_output"] = 0.0
        return out

    fc, fr = cutoff_coeff(cutoff, res)
    cur_env=env; cur_lfo=lfo_ph; cfz=fz; cfz2=fz2; cur_freq=freq_cur; cur_out=last_out
    freq_alpha = 1.0 - np.exp(-1.0 / max(1.0, DECLICK_TIME * SAMPLE_RATE))
    out_alpha = 1.0 - np.exp(-1.0 / max(1.0, 0.0015 * SAMPLE_RATE))
    mod_alpha = 1.0 - np.exp(-1.0 / max(1.0, MOD_SMOOTH_TIME * SAMPLE_RATE))
    param_alpha = 1.0 - np.exp(-1.0 / max(1.0, PARAM_SMOOTH_TIME * SAMPLE_RATE))
    filter_alpha = 1.0 - np.exp(-1.0 / max(1.0, FILTER_SMOOTH_TIME * SAMPLE_RATE))
    active_level_sq = sum(max(0.0, osc["level"]) ** 2 for osc in oscillators)
    osc_norm = 1.0 / max(1.0, np.sqrt(active_level_sq))

    lfo_vals = np.empty(frames, dtype=np.float32)
    lfo_depths = np.empty(frames, dtype=np.float32)
    env_vals = np.empty(frames, dtype=np.float32)
    freq_vals = np.empty(frames, dtype=np.float32)
    vol_vals = np.empty(frames, dtype=np.float32)
    filter_mixes = np.empty(frames, dtype=np.float32)
    filter_target = 1.0 if filt_on else 0.0

    for i in range(frames):
        lfo_dep += (lfo_dep_target - lfo_dep) * mod_alpha
        vol_cur += (vol_target - vol_cur) * param_alpha
        filter_mix += (filter_target - filter_mix) * filter_alpha
        lp = 2*np.pi*cur_lfo
        if   lfo_wf==0: lv=float(np.sin(lp))
        elif lfo_wf==1: lv=2*abs(2*(cur_lfo%1)-1)-1
        else:           lv=float(np.sign(np.sin(lp)))
        cur_lfo += lfo_rate/SAMPLE_RATE

        if gate_open: cur_env += (1.0-cur_env)/(atk*SAMPLE_RATE)
        else:         cur_env -= cur_env/(rel*SAMPLE_RATE)
        cur_env = max(0.0,min(1.0,cur_env))

        freq_goal = target_freq*(2.0**(lv*lfo_dep) if lfo_tgt==0 else 1.0)
        cur_freq += (freq_goal - cur_freq) * freq_alpha

        lfo_vals[i] = lv
        lfo_depths[i] = lfo_dep
        env_vals[i] = cur_env
        freq_vals[i] = cur_freq
        vol_vals[i] = vol_cur
        filter_mixes[i] = filter_mix

    smp = np.zeros(frames, dtype=np.float32)
    if voices == 1:
        voice_ratios = np.array([1.0], dtype=np.float32)
    else:
        center = (voices - 1) / 2.0
        spread = 0.18
        voice_offsets = ((np.arange(voices, dtype=np.float32) - center) / center) * spread
        voice_ratios = (2.0 ** (voice_offsets / 12.0)).astype(np.float32, copy=False)

    for osc in oscillators:
        osc_level = max(0.0, osc["level"])
        if osc_level <= 0.0001:
            continue

        base_ratio = float((2.0 ** osc["octave"]) * (2.0 ** (osc["detune_cents"] / 1200.0)))
        base_steps = (freq_vals * (base_ratio / SAMPLE_RATE)).astype(np.float32, copy=False)
        base_prefix = np.empty(frames, dtype=np.float32)
        base_prefix[0] = 0.0
        if frames > 1:
            base_prefix[1:] = np.cumsum(base_steps[:-1], dtype=np.float64)
        total_base_step = float(np.sum(base_steps, dtype=np.float64))

        phases = osc["voice_phases"]
        osc_smp = np.zeros(frames, dtype=np.float32)
        for vi in range(voices):
            ratio = float(voice_ratios[vi])
            phase_path = phases[vi] + base_prefix * ratio
            phase_steps = base_steps * ratio
            osc_smp += osc_block(phase_path, osc["waveform"], phase_steps)
            phases[vi] = (phases[vi] + total_base_step * ratio) % 1.0
        if voices > 1:
            osc_smp /= voices
        smp += osc_smp * osc_level

    smp *= osc_norm

    if lfo_tgt == 1:
        ev = vol_vals * ((1.0 - lfo_depths) + lfo_depths * (0.5 + 0.5 * lfo_vals))
        ev = np.clip(ev, 0.0, 1.0).astype(np.float32, copy=False)
    else:
        ev = vol_vals

    smp = smp * env_vals * ev * 0.52
    out = np.empty(frames, dtype=np.float32)

    cutoff_mod = np.clip(cutoff + lfo_vals * lfo_depths * 0.85, 0.0, 1.0).astype(np.float32, copy=False) if lfo_tgt == 2 else None
    for i in range(frames):
        dry = float(smp[i])
        ec = float(cutoff_mod[i]) if cutoff_mod is not None else cutoff
        fc2 = np.exp(-2*np.pi*(20*(20000/20)**ec)/SAMPLE_RATE)
        fb  = min(fr,0.95)*(cfz-cfz2)
        hp  = dry - cfz - fb
        cfz  = cfz  * fc2 + hp*(1-fc2)
        cfz2 = cfz2 * fc2 + cfz*(1-fc2)
        wet  = soft_saturate_sample(cfz2, drive=0.85, ceiling=1.0)
        mixed = dry * (1.0 - filter_mixes[i]) + wet * filter_mixes[i]
        cur_out += (mixed - cur_out) * out_alpha
        out[i] = cur_out

    with synth_lock:
        for osc_idx, osc in enumerate(oscillators):
            synth["oscillators"][osc_idx]["voice_phases"] = osc["voice_phases"]
        synth["env"]=cur_env
        synth["lfo_depth_current"]=lfo_dep
        synth["lfo_phase"]=cur_lfo%1.0
        synth["volume_current"] = vol_cur
        synth["filter_mix"] = filter_mix
        synth["filt_z"]=cfz; synth["filt_z2"]=cfz2
        synth["freq_current"] = cur_freq
        synth["last_output"] = cur_out
    return out

delay_buf = np.zeros(SAMPLE_RATE, dtype=np.float32)
delay_idx = 0
reverb_buf = np.zeros(int(SAMPLE_RATE * 1.4), dtype=np.float32)
reverb_idx = 0
fx_tone_state = {"warm_lp": 0.0, "air_lp": 0.0, "reverb_damp": 0.0}
REVERB_TAPS = [int(SAMPLE_RATE * t) for t in (0.113, 0.173, 0.229, 0.317)]
synth_dc_state = {"x1": 0.0, "y1": 0.0}
master_dc_state = {"x1": 0.0, "y1": 0.0}
master_limiter_state = {"gain": 1.0}

def apply_fx(signal):
    global delay_idx, reverb_idx
    with synth_lock:
        drive_target    = synth["fx_drive"]
        mix_target      = synth["fx_delay_mix"]
        feedback_target = synth["fx_delay_feedback"]
        dtime_target    = synth["fx_delay_time"]
        warmth_target   = synth["fx_warmth"]
        air_target      = synth["fx_air"]
        reverb_target   = synth["fx_reverb"]
        drive_cur       = synth["fx_drive_current"]
        mix_cur         = synth["fx_delay_mix_current"]
        feedback_cur    = synth["fx_delay_feedback_current"]
        dtime_cur       = synth["fx_delay_time_current"]
        warmth_cur      = synth["fx_warmth_current"]
        air_cur         = synth["fx_air_current"]
        reverb_cur      = synth["fx_reverb_current"]

    if max(
        drive_target, mix_target, warmth_target, air_target, reverb_target,
        drive_cur, mix_cur, warmth_cur, air_cur, reverb_cur,
    ) <= 0.0001:
        return signal

    out = np.empty_like(signal)
    fx_alpha = 1.0 - np.exp(-1.0 / max(1.0, FX_SMOOTH_TIME * SAMPLE_RATE))
    delay_len = len(delay_buf)
    reverb_len = len(reverb_buf)
    warm_lp = fx_tone_state["warm_lp"]
    air_lp = fx_tone_state["air_lp"]
    reverb_damp = fx_tone_state["reverb_damp"]

    for i, dry in enumerate(signal):
        drive_cur += (drive_target - drive_cur) * fx_alpha
        mix_cur += (mix_target - mix_cur) * fx_alpha
        feedback_cur += (feedback_target - feedback_cur) * fx_alpha
        dtime_cur += (dtime_target - dtime_cur) * fx_alpha
        warmth_cur += (warmth_target - warmth_cur) * fx_alpha
        air_cur += (air_target - air_cur) * fx_alpha
        reverb_cur += (reverb_target - reverb_cur) * fx_alpha

        warm_lp += (dry - warm_lp) * (0.018 + warmth_cur * 0.05)
        warmed = dry * (1.0 - warmth_cur * 0.28) + warm_lp * warmth_cur * 0.28

        gain = 1.0 + drive_cur * 8.0
        norm = max(np.tanh(gain), 1e-6)
        delay_samples = max(1, int((0.08 + dtime_cur * 0.72) * SAMPLE_RATE))
        driven = np.tanh(warmed * gain) / norm
        if warmth_cur > 0.0001:
            driven = soft_saturate_sample(driven + (warm_lp - driven) * warmth_cur * 0.18, drive=1.0 + warmth_cur * 0.6, ceiling=0.96)

        air_lp += (driven - air_lp) * 0.14
        airy = driven + (driven - air_lp) * air_cur * 0.55

        wet = delay_buf[(delay_idx - delay_samples) % delay_len]
        delayed = airy * (1.0 - mix_cur) + wet * mix_cur
        fed = airy * 0.82 + wet * feedback_cur
        delay_buf[delay_idx] = soft_saturate_sample(fed, drive=1.02, ceiling=0.9)

        tap1 = reverb_buf[(reverb_idx - REVERB_TAPS[0]) % reverb_len]
        tap2 = reverb_buf[(reverb_idx - REVERB_TAPS[1]) % reverb_len]
        tap3 = reverb_buf[(reverb_idx - REVERB_TAPS[2]) % reverb_len]
        tap4 = reverb_buf[(reverb_idx - REVERB_TAPS[3]) % reverb_len]
        reverb_wet = (tap1 + tap2 * 0.85 + tap3 * 0.72 + tap4 * 0.58) / 3.15
        reverb_damp += (reverb_wet - reverb_damp) * 0.08
        reverb_feed = airy * (0.18 + reverb_cur * 0.08) + reverb_damp * (0.72 + reverb_cur * 0.18)
        reverb_buf[reverb_idx] = soft_saturate_sample(reverb_feed, drive=1.01, ceiling=0.9)

        out[i] = delayed * (1.0 - reverb_cur * 0.42) + reverb_wet * reverb_cur * 0.42
        delay_idx = (delay_idx + 1) % delay_len
        reverb_idx = (reverb_idx + 1) % reverb_len

    with synth_lock:
        synth["fx_drive_current"] = drive_cur
        synth["fx_delay_mix_current"] = mix_cur
        synth["fx_delay_feedback_current"] = feedback_cur
        synth["fx_delay_time_current"] = dtime_cur
        synth["fx_warmth_current"] = warmth_cur
        synth["fx_air_current"] = air_cur
        synth["fx_reverb_current"] = reverb_cur

    fx_tone_state["warm_lp"] = warm_lp
    fx_tone_state["air_lp"] = air_lp
    fx_tone_state["reverb_damp"] = reverb_damp

    return out * 0.95

# ═══════════════════════════════════════════════════════════════════════════════
#  AUDIO CALLBACK
# ═══════════════════════════════════════════════════════════════════════════════
_sample_clock = 0

def audio_cb(outdata, frames, t, status):
    global _sample_clock
    if status:
        with synth_lock:
            synth["xruns"] += 1

    synth_live = dc_block(apply_fx(gen_synth(frames)), synth_dc_state)
    loop_out = process_loop(synth_live)

    drum_out = np.zeros(frames, dtype=np.float32)
    with drum_lock:
        pending_trigs = drum["trig"][:]
        drum["trig"] = [False] * NUM_DRUM_VOICES
        drum_vols = drum["vol"][:]
        drum_bank = current_drum_bank_locked()

    for v, pending in enumerate(pending_trigs):
        if pending:
            reset_drum_voice(v)
            drum_trig_sample[v] = _sample_clock

    for v in range(NUM_DRUM_VOICES):
        if drum_trig_sample[v] < 0:
            continue
        age = _sample_clock - drum_trig_sample[v]
        if age >= drum_bank["lengths"][v]:
            drum_trig_sample[v] = -1
            continue
        remaining = min(frames, drum_bank["lengths"][v] - age)
        chunk = DRUM_SYNTHS[v](drum_bank, age, remaining)
        drum_out[:remaining] += chunk * drum_vols[v] * 0.6

    drum_out *= 0.62
    synth_bus = (synth_live + loop_out) * MASTER_HEADROOM
    mixed = synth_bus + drum_out * 0.5
    mixed = dc_block(mixed, master_dc_state)
    mixed = apply_master_limiter(mixed, master_limiter_state)
    mixed *= OUTPUT_GAIN
    np.clip(mixed, -LIMIT_CEILING, LIMIT_CEILING, out=mixed)
    update_reactive_state(frames, synth_bus, drum_out, mixed, pending_trigs)
    with global_rec_lock:
        if global_rec["recording"]:
            global_rec["chunks"].append(mixed.copy())
    scope_show_drums = ui_state["scope_show_drums"]
    scope_sig = mixed if scope_show_drums else synth_bus

    outdata[:,0] = mixed
    if outdata.shape[1] > 1: outdata[:,1] = mixed

    with scope_lock:
        scope_buf.extend(scope_sig)
    _sample_clock += frames

def apply_drum_pattern_locked(pattern_idx):
    drum["steps"] = [[False] * NUM_STEPS for _ in range(NUM_DRUM_VOICES)]
    if pattern_idx == 0:
        for step in (0, 8, 16, 24):
            drum["steps"][0][step] = True
        for step in (4, 12, 20, 28):
            drum["steps"][1][step] = True
        for step in (12, 28):
            drum["steps"][2][step] = True
        for step in range(0, NUM_STEPS, 2):
            drum["steps"][3][step] = True
        for step in (6, 14, 22, 30):
            drum["steps"][4][step] = True
        for step in (10, 26):
            drum["steps"][5][step] = True
    else:
        for step in (0, 11, 16, 24):
            drum["steps"][0][step] = True
        for step in (4, 12, 20, 28):
            drum["steps"][1][step] = True
        for step in (10, 26):
            drum["steps"][2][step] = True
        for step in range(NUM_STEPS):
            drum["steps"][3][step] = (step % 2 == 0)
        for step in (7, 15, 23, 31):
            drum["steps"][4][step] = True
        for step in (6, 22, 30):
            drum["steps"][5][step] = True

# ═══════════════════════════════════════════════════════════════════════════════
#  SEQUENCER THREAD
# ═══════════════════════════════════════════════════════════════════════════════
def sequencer_thread():
    while True:
        with drum_lock:
            running = drum["running"]
            bpm     = drum["bpm"]
            step    = drum["cur_step"]
            steps   = [drum["steps"][v][step] for v in range(NUM_DRUM_VOICES)]

        if running:
            # fire triggers for active steps
            with drum_lock:
                for v in range(NUM_DRUM_VOICES):
                    if steps[v]:
                        drum["trig"][v] = True
                drum["cur_step"] = (step + 1) % NUM_STEPS

            beat_dur = 60.0 / bpm / 4.0   # 16th notes across 32 steps
            time.sleep(beat_dur)
        else:
            time.sleep(0.05)

seq_thread = threading.Thread(target=sequencer_thread, daemon=True)
seq_thread.start()

# ═══════════════════════════════════════════════════════════════════════════════
#  BRAILLE SCOPE
# ═══════════════════════════════════════════════════════════════════════════════
BRAILLE_BASE = 0x2800
BD = [[0x01,0x02,0x04,0x40],[0x08,0x10,0x20,0x80]]

def camera_command():
    ffmpeg = shutil.which("ffmpeg")
    if not ffmpeg:
        return None
    return [
        ffmpeg,
        "-loglevel", "quiet",
        "-f", "avfoundation",
        "-framerate", str(CAMERA_CAPTURE_FPS),
        "-video_size", "640x480",
        "-i", CAMERA_DEVICE,
        "-vf", f"fps={CAMERA_OUTPUT_FPS},format=gray,eq=contrast=2.5:brightness=0.05,scale={CAMERA_FRAME_WIDTH}:{CAMERA_FRAME_HEIGHT}",
        "-f", "rawvideo",
        "-pix_fmt", "gray",
        "pipe:1",
    ]

def stop_camera_stream():
    with camera_lock:
        thread = camera_state["thread"]
        proc = camera_state["process"]
        if thread is None and proc is None and not camera_state["running"]:
            camera_state["gray"] = None
            camera_state["error"] = "Camera idle"
            return
        camera_state["stop_event"].set()
        camera_state["running"] = False
        camera_state["thread"] = None
        camera_state["process"] = None
    if proc is not None:
        try:
            proc.terminate()
            proc.wait(timeout=1.0)
        except Exception:
            try:
                proc.kill()
            except Exception:
                pass
    if thread is not None and thread is not threading.current_thread():
        thread.join(timeout=1.0)
    with camera_lock:
        camera_state["gray"] = None
        camera_state["error"] = "Camera idle"

def camera_worker():
    cmd = camera_command()
    if cmd is None:
        with camera_lock:
            camera_state["running"] = False
            camera_state["error"] = "ffmpeg not found"
        return

    frame_size = CAMERA_FRAME_WIDTH * CAMERA_FRAME_HEIGHT
    proc = None
    try:
        proc = subprocess.Popen(cmd, stdout=subprocess.PIPE, stderr=subprocess.DEVNULL)
        with camera_lock:
            camera_state["process"] = proc
            camera_state["error"] = "Starting camera..."
        stdout = proc.stdout
        if stdout is None:
            raise RuntimeError("Camera pipe unavailable")

        while not camera_state["stop_event"].is_set():
            buf = bytearray()
            while len(buf) < frame_size and not camera_state["stop_event"].is_set():
                chunk = stdout.read(frame_size - len(buf))
                if not chunk:
                    raise RuntimeError("Camera stream ended")
                buf.extend(chunk)
            if len(buf) < frame_size:
                break
            gray = np.frombuffer(bytes(buf), dtype=np.uint8).reshape((CAMERA_FRAME_HEIGHT, CAMERA_FRAME_WIDTH))
            with camera_lock:
                camera_state["gray"] = gray
                camera_state["error"] = ""
    except Exception as exc:
        with camera_lock:
            camera_state["gray"] = None
            camera_state["error"] = short_label(str(exc), 48)
    finally:
        if proc is not None:
            try:
                proc.terminate()
                proc.wait(timeout=0.5)
            except Exception:
                try:
                    proc.kill()
                except Exception:
                    pass
        with camera_lock:
            camera_state["process"] = None
            camera_state["thread"] = None
            camera_state["running"] = False

def ensure_camera_stream_running():
    with camera_lock:
        if camera_state["running"] and camera_state["thread"] is not None:
            return
        camera_state["stop_event"].clear()
        camera_state["running"] = True
        thread = threading.Thread(target=camera_worker, daemon=True)
        camera_state["thread"] = thread
    thread.start()

def compress_visual_segments(chars, attrs):
    if not chars:
        return []
    segments = []
    start = 0
    cur_attr = attrs[0]
    for idx in range(1, len(chars)):
        if attrs[idx] != cur_attr:
            segments.append(("".join(chars[start:idx]), cur_attr))
            start = idx
            cur_attr = attrs[idx]
    segments.append(("".join(chars[start:]), cur_attr))
    return segments

def update_reactive_state(frames, synth_bus, drum_out, mixed, pending_trigs):
    if frames <= 0:
        return
    decay = float(np.exp(-frames / (SAMPLE_RATE * 0.18)))
    synth_level = clamp(float(np.sqrt(np.mean(np.square(synth_bus)))) * 3.2, 0.0, 1.0)
    drum_level = clamp(float(np.sqrt(np.mean(np.square(drum_out)))) * 4.5, 0.0, 1.0)
    master_level = clamp(float(np.sqrt(np.mean(np.square(mixed)))) * 3.2, 0.0, 1.0)
    with synth_lock:
        note_level = clamp(float(synth["env"]), 0.0, 1.0)

    with reactive_lock:
        for key in reactive_state:
            reactive_state[key] *= decay
        reactive_state["synth"] = max(reactive_state["synth"], synth_level)
        reactive_state["drums"] = max(reactive_state["drums"], drum_level)
        reactive_state["master"] = max(reactive_state["master"], master_level)
        reactive_state["note"] = max(reactive_state["note"], note_level)
        if pending_trigs[0]:
            reactive_state["kick"] = 1.0
        if pending_trigs[1] or pending_trigs[2]:
            reactive_state["snare"] = 1.0
        if pending_trigs[3] or pending_trigs[5]:
            reactive_state["hat"] = 1.0

def render_scope(samples, width, height):
    dw, dh = width * 2, height * 4
    idx = np.linspace(0, len(samples)-1, dw).astype(int)
    sig = np.clip(np.array(samples)[idx], -1, 1)
    rows = ((1.0 - sig) / 2.0 * (dh - 1)).astype(int)
    grid = [[0]*width for _ in range(height)]

    def plot_dot(dx, dy):
        cx,cy,sx,sy = dx//2, dy//4, dx%2, dy%4
        if 0<=cx<width and 0<=cy<height:
            grid[cy][cx] |= BD[sx][sy]

    # draw vertical line segments connecting consecutive samples
    for dx in range(dw):
        y0 = rows[dx]
        y1 = rows[dx+1] if dx+1 < dw else y0
        lo, hi = (y0,y1) if y0<=y1 else (y1,y0)
        for dy in range(lo, hi+1):
            plot_dot(dx, dy)

    # zero line
    zy = dh // 2
    zcy, zsy = zy//4, zy%4
    for cx in range(width):
        if grid[zcy][cx] == 0:
            grid[zcy][cx] |= BD[0][zsy]

    return ["".join(chr(BRAILLE_BASE|grid[r][c]) for c in range(width))
            for r in range(height)]

def render_camera_visual(width, height, style_idx, depth, C, scope_attr, B, DIM):
    ensure_camera_stream_running()
    with camera_lock:
        gray = None if camera_state["gray"] is None else camera_state["gray"].copy()
        err = camera_state["error"]
    with reactive_lock:
        rx = reactive_state.copy()
    if gray is None or gray.size == 0:
        msg = short_label(err or "Starting camera...", max(1, width - 2))
        lines = [" " * width for _ in range(height)]
        if height > 0:
            row = height // 2
            pad = max(0, (width - len(msg)) // 2)
            lines[row] = (" " * pad + msg)[:width].ljust(width)
        return lines

    y_idx = np.linspace(0, gray.shape[0] - 1, max(1, height)).astype(int)
    x_idx = np.linspace(0, gray.shape[1] - 1, max(1, width)).astype(int)
    small = gray[np.ix_(y_idx, x_idx)].astype(np.float32) / 255.0
    style_idx = int(clamp(style_idx, 0, len(CAMERA_REACTIVE_STYLES) - 1))
    depth = clamp(depth, 0.0, 1.0)
    kick = rx["kick"] * depth
    synth_amt = rx["synth"] * depth
    drums = rx["drums"] * depth
    note_amt = rx["note"] * depth
    hat = rx["hat"] * depth
    master = rx["master"] * depth
    t = time.time()

    base_attr = scope_attr
    hi_attr = (C[6] | B)
    alt_attr = (C[7] | B)
    hot_attr = (C[9] | B)
    cool_attr = (C[5] | B)
    chars_small = small.copy()
    attrs = np.full((small.shape[0], small.shape[1]), base_attr, dtype=object)
    ramp = np.array(list(" .:-=+*#%@"))

    if style_idx == 1:
        chars_small = np.clip(chars_small + kick * 0.35, 0.0, 1.0)
        if kick > 0.08:
            attrs[:,:] = hi_attr if kick < 0.55 else hot_attr
    elif style_idx == 2:
        band = max(1, int(width * (0.12 + 0.25 * synth_amt)))
        mid = width // 2
        lo = max(0, mid - band)
        hi = min(width, mid + band)
        attrs[:, lo:hi] = cool_attr
        chars_small[:, lo:hi] = np.clip(chars_small[:, lo:hi] + synth_amt * 0.28 + note_amt * 0.22, 0.0, 1.0)
    elif style_idx == 3:
        chars_small = np.where(drums > 0.16, 1.0 - chars_small * (0.65 - drums * 0.2), chars_small)
        if drums > 0.12:
            attrs[:,:] = hi_attr
    elif style_idx == 4:
        scan_x = int(((t * 9.0) + kick * 12.0) % max(1, width))
        span = max(1, int(2 + master * 8))
        lo = max(0, scan_x - span)
        hi = min(width, scan_x + span + 1)
        attrs[:, lo:hi] = alt_attr
        chars_small[:, lo:hi] = np.clip(chars_small[:, lo:hi] + 0.25 + synth_amt * 0.3, 0.0, 1.0)
    elif style_idx == 5:
        edge = np.abs(np.diff(chars_small, axis=1, prepend=chars_small[:, :1]))
        chars_small = np.clip(chars_small * 0.35 + edge * (1.2 + kick * 1.5), 0.0, 1.0)
        attrs[edge > (0.18 - kick * 0.08)] = hot_attr if kick > 0.2 else hi_attr
        ramp = np.array(list(" .'^:!*ox%#@"))
    elif style_idx == 6:
        shift = int(round((kick * 5.0) + (drums * 2.0)))
        if shift > 0:
            for row in range(chars_small.shape[0]):
                chars_small[row] = np.roll(chars_small[row], shift if row % 2 == 0 else -shift)
        attrs[::2, :] = alt_attr if kick > 0.1 else base_attr
    elif style_idx == 7:
        levels_q = max(2, int(3 + note_amt * 5 + synth_amt * 3))
        chars_small = np.floor(chars_small * levels_q) / levels_q
        attrs[chars_small > 0.55] = cool_attr
        ramp = np.array(list("  .:=+*#%@"))
    elif style_idx == 8:
        chars_small = np.clip(chars_small + kick * 0.32 + drums * 0.18, 0.0, 1.0)
        attrs[chars_small > 0.72] = hot_attr
        attrs[(chars_small > 0.46) & (chars_small <= 0.72)] = hi_attr
        ramp = np.array(list(" .,:;irsXA253hMHGS#9B&@"))
    elif style_idx == 9:
        chars_small = np.clip(chars_small * (0.85 + synth_amt * 0.25) + hat * 0.12, 0.0, 1.0)
        attrs[chars_small > 0.62] = alt_attr
        attrs[chars_small > 0.82] = cool_attr
        ramp = np.array(list("  .-~=+*#%@"))
    elif style_idx == 10:
        third = max(1, width // 3)
        attrs[:, :third] = alt_attr
        attrs[:, third:third*2] = cool_attr
        attrs[:, third*2:] = hi_attr
        if synth_amt > 0.1:
            attrs[:, max(0, width//2 - 2):min(width, width//2 + 2)] = hot_attr
        chars_small = np.clip(chars_small + master * 0.12, 0.0, 1.0)
    elif style_idx == 11:
        cols = np.linspace(0, 1, width, dtype=np.float32)
        rain = (np.sin(cols * 19.0 + t * (6.0 + drums * 5.0)) * 0.5 + 0.5) * (0.25 + master * 0.55)
        chars_small = np.clip(chars_small * 0.5 + rain[None, :], 0.0, 1.0)
        attrs[chars_small > 0.58] = cool_attr
        attrs[chars_small > 0.8] = hi_attr
        ramp = np.array(list(" .`:,;1!i><+*#%@"))

    levels = (chars_small * (len(ramp) - 1)).astype(int)
    levels = np.clip(levels, 0, len(ramp) - 1)
    out_rows = []
    for row_idx in range(levels.shape[0]):
        row_chars = [str(ch) for ch in ramp[levels[row_idx]]]
        row_attrs = list(attrs[row_idx])
        out_rows.append(compress_visual_segments(row_chars, row_attrs))
    return out_rows

# ═══════════════════════════════════════════════════════════════════════════════
#  UI HELPERS
# ═══════════════════════════════════════════════════════════════════════════════
def safe_addstr(scr, y, x, s, attr=0):
    h,w = scr.getmaxyx()
    if y<0 or y>=h or x<0 or x>=w: return
    try: scr.addstr(y, x, s[:w-x], attr)
    except curses.error: pass

def hbar(v, w, fc="█", ec="░"): n=int(v*w); return fc*n+ec*(w-n)

def draw_box(scr, y, x, width, height, title, border_attr=0, fill_attr=0):
    if width < 4 or height < 3:
        return
    top = "┌" + "─" * (width - 2) + "┐"
    bot = "└" + "─" * (width - 2) + "┘"
    safe_addstr(scr, y, x, top, border_attr)
    for row in range(1, height - 1):
        safe_addstr(scr, y + row, x, "│" + " " * (width - 2) + "│", fill_attr)
    safe_addstr(scr, y + height - 1, x, bot, border_attr)
    safe_addstr(scr, y, x + 2, f" {title} ", border_attr)

def draw_keycaps(scr, y, x, labels, key_attr, gap=1):
    cx = x
    for label in labels:
        token = f"[{label}]"
        safe_addstr(scr, y, cx, token, key_attr)
        cx += len(token) + gap
    return cx

def wrap_text(text, width):
    width = max(1, int(width))
    return textwrap.wrap(text, width=width, break_long_words=False, break_on_hyphens=False) or [""]

def draw_help_section(scr, y, x, width, title, rows, scope_attr, C, B, DIM):
    if width < 18:
        return y

    key_w = max(12, min(24, width // 3))
    desc_w = max(8, width - key_w - 3)

    safe_addstr(scr, y, x, f" {title} ", C[1]|B)
    safe_addstr(scr, y + 1, x, "─" * max(0, width - 1), scope_attr)
    row_y = y + 2

    for labels, desc in rows:
        key_text = " ".join(f"[{label}]" for label in labels)
        key_lines = wrap_text(key_text, key_w)
        desc_lines = wrap_text(desc, desc_w)
        row_h = max(len(key_lines), len(desc_lines))

        for i in range(row_h):
            if i < len(key_lines):
                safe_addstr(scr, row_y + i, x, key_lines[i], C[8]|B)
            if i == 0:
                safe_addstr(scr, row_y + i, x + key_w, "→", C[6]|B)
            if i < len(desc_lines):
                safe_addstr(scr, row_y + i, x + key_w + 2, desc_lines[i], C[3])

        row_y += row_h + 1

    return row_y

def draw_help_overlay(scr, h, w, scope_attr, C, B, DIM):
    box_w = min(92, max(54, w - 4))
    box_h = min(26, max(16, h - 2))
    box_x = max(1, (w - box_w) // 2)
    box_y = max(1, (h - box_h) // 2)
    inner_w = box_w - 4
    left_x = box_x + 3

    draw_box(scr, box_y, box_x, box_w, box_h, "MURSYNTH HELP", scope_attr|B)
    safe_addstr(scr, box_y + 1, left_x, "Terminal synth + drums + looper", C[3]|B)
    if box_w >= 72:
        safe_addstr(scr, box_y + 1, box_x + box_w - 28, "Hold H to keep this visible", C[6])
    else:
        safe_addstr(scr, box_y + 2, left_x, "Hold H to keep this visible", C[6])
    safe_addstr(scr, box_y + 3, left_x, "─" * inner_w, scope_attr)

    sections = [
        ("PLAY NOTES", [
            (["a", "w", "s", "e", "d", "f", "t", "g", "y", "h", "u", "j", "k"], "Chromatic keyboard relative to the current root."),
            (["MIDI keys"], "External MIDI notes play the synth; note remaps live in settings."),
            (["←", "→"], "Move the base/root note down or up."),
            (["SPC"], "Release the currently held note."),
        ]),
        ("OSC + SHAPE", [
            (["1", "2"], "Select oscillator 1 or 2 for editing."),
            (["z", "x"], "Change waveform of the selected oscillator."),
            (["↑", "↓"], "Adjust master volume."),
            (["[", "]", "{", "}"], "Adjust attack and release."),
            (["m"], "Toggle gate/free mode."),
        ]),
        ("MOD + FILTER + FX", [
            (["l", "o"], "Change LFO wave and target."),
            ([",", ".", ";", "'"], "Adjust LFO rate and depth."),
            (["p", "-", "=", "_", "+"], "Toggle filter, adjust cutoff and resonance."),
            (["D", "F", "J", "K", "N", "M", "V", "B"], "Drive, delay mix, feedback, and delay time."),
            (["S + ←→"], "In settings, adjust visuals, camera-reactive styles, warmth, air, and reverb."),
        ]),
        ("LOOP + DRUMS", [
            (["R", "T", "Y", "P", "U"], "Record, overdub, undo last overdub layer, toggle playback, or clear the loop."),
            (["G"], "Start or stop global mix recording to a WAV file."),
            (["TAB"], "Switch between synth focus and drum sequencer focus."),
            (["←", "→", "↑", "↓"], "Move around the drum grid while sequencer focus is active."),
            (["SPC", "r", "c", "X", "1", "2"], "Toggle step, run, clear row/all with double-press confirm, or load a 32-step pattern."),
            (["S"], "Open settings for synth, camera FX, MIDI device selection, note remaps, and pad/knob learn."),
        ]),
    ]

    content_y = box_y + 4
    use_two_cols = inner_w >= 68 and box_h >= 18

    if use_two_cols:
        col_gap = 4
        col_w = (inner_w - col_gap) // 2
        right_x = left_x + col_w + col_gap
        left_y = content_y
        right_y = content_y

        left_y = draw_help_section(scr, left_y, left_x, col_w, sections[0][0], sections[0][1], scope_attr, C, B, DIM)
        left_y = draw_help_section(scr, left_y, left_x, col_w, sections[2][0], sections[2][1], scope_attr, C, B, DIM)
        right_y = draw_help_section(scr, right_y, right_x, col_w, sections[1][0], sections[1][1], scope_attr, C, B, DIM)
        right_y = draw_help_section(scr, right_y, right_x, col_w, sections[3][0], sections[3][1], scope_attr, C, B, DIM)
        footer_y = max(left_y, right_y)
    else:
        footer_y = content_y
        for title, rows in sections:
            footer_y = draw_help_section(scr, footer_y, left_x, inner_w, title, rows, scope_attr, C, B, DIM)

    footer_y += 1
    safe_addstr(scr, footer_y, left_x, "Tip", C[1]|B)
    for i, line in enumerate(wrap_text("Use S for deeper oscillator editing and TAB to jump between synth and drums.", inner_w - 6)):
        safe_addstr(scr, footer_y + i, left_x + 6, line, C[3])

    goal_y = footer_y + max(1, len(wrap_text("Use S for deeper oscillator editing and TAB to jump between synth and drums.", inner_w - 6)))
    safe_addstr(scr, goal_y, left_x, "Goal", C[1]|B)
    for i, line in enumerate(wrap_text("Layer OSC1 + OSC2 for bass/lead blends, then capture phrases with the loop controls.", inner_w - 6)):
        safe_addstr(scr, goal_y + i, left_x + 6, line, C[6])

def settings_row_count(page):
    return [14, 7, 7, 9, 8, 7][page]

def selected_midi_bind_target_locked():
    return MIDI_BIND_TARGETS[midi["learn_target_index"]]

def selected_midi_binding_locked():
    target_id, label, kind = selected_midi_bind_target_locked()
    if kind == "cc":
        return label, kind, midi["cc_bindings"].get(target_id)
    return label, kind, midi["note_bindings"].get(target_id)

def refresh_audio_devices_locked():
    try:
        devices = sd.query_devices()
        default_input, default_output = sd.default.device
    except Exception as exc:
        audio["inputs"] = []
        audio["outputs"] = []
        audio["input_index"] = 0
        audio["output_index"] = 0
        audio["input_name"] = ""
        audio["output_name"] = ""
        audio["status"] = short_label(f"Audio query failed: {exc}", 42)
        return

    inputs = []
    outputs = []
    for device_index, device_info in enumerate(devices):
        device_name = short_label(str(device_info.get("name", f"Device {device_index}")).replace("\n", " ").strip(), 42)
        if int(device_info.get("max_input_channels", 0)) > 0:
            inputs.append({"id": device_index, "name": device_name})
        if int(device_info.get("max_output_channels", 0)) > 0:
            outputs.append({"id": device_index, "name": device_name})

    audio["inputs"] = inputs
    audio["outputs"] = outputs

    def sync_selection(devices_list, index_key, name_key, default_device_id):
        current_name = audio[name_key]
        current_index = audio[index_key]
        selected_pos = None
        if devices_list:
            for pos, device_info in enumerate(devices_list):
                if current_name and device_info["name"] == current_name:
                    selected_pos = pos
                    break
            if selected_pos is None:
                for pos, device_info in enumerate(devices_list):
                    if device_info["id"] == default_device_id:
                        selected_pos = pos
                        break
            if selected_pos is None:
                selected_pos = int(clamp(current_index, 0, len(devices_list) - 1))
            audio[index_key] = selected_pos
            audio[name_key] = devices_list[selected_pos]["name"]
        else:
            audio[index_key] = 0
            audio[name_key] = ""

    sync_selection(inputs, "input_index", "input_name", default_input)
    sync_selection(outputs, "output_index", "output_name", default_output)

    if not outputs:
        audio["status"] = "No audio output devices"
    else:
        audio["status"] = f"Audio {len(outputs)} out / {len(inputs)} in"

def selected_audio_input_locked():
    if 0 <= audio["input_index"] < len(audio["inputs"]):
        return audio["inputs"][audio["input_index"]]
    return None

def selected_audio_output_locked():
    if 0 <= audio["output_index"] < len(audio["outputs"]):
        return audio["outputs"][audio["output_index"]]
    return None

def open_audio_stream():
    with audio_lock:
        refresh_audio_devices_locked()
        output_device = selected_audio_output_locked()
        output_device_id = None if output_device is None else output_device["id"]
    try:
        stream = sd.OutputStream(
            samplerate=SAMPLE_RATE,
            blocksize=BLOCK_SIZE,
            channels=2,
            dtype='float32',
            latency=0.2,
            device=output_device_id,
            callback=audio_cb,
        )
        stream.start()
        with audio_lock:
            active_name = output_device["name"] if output_device is not None else "default"
            audio["status"] = short_label(f"Output active: {active_name}", 42)
        return stream
    except Exception as exc:
        with audio_lock:
            audio["status"] = short_label(f"Audio open failed: {exc}", 42)
        return None

def close_audio_stream(stream):
    if stream is None:
        return
    try:
        stream.stop()
    except Exception:
        pass
    try:
        stream.close()
    except Exception:
        pass

# ═══════════════════════════════════════════════════════════════════════════════
#  MAIN DRAW LOOP
# ═══════════════════════════════════════════════════════════════════════════════
def draw(stdscr):
    curses.curs_set(0); stdscr.timeout(INPUT_POLL_MS); stdscr.keypad(True)
    try:
        curses.set_escdelay(ESC_KEY_DELAY_MS)
    except AttributeError:
        pass
    curses.start_color(); curses.use_default_colors()
    curses.init_pair(1,  curses.COLOR_BLACK,   curses.COLOR_CYAN)    # header
    curses.init_pair(2,  curses.COLOR_CYAN,    -1)                   # label
    curses.init_pair(3,  curses.COLOR_WHITE,   -1)                   # value
    curses.init_pair(4,  curses.COLOR_BLACK,   curses.COLOR_GREEN)   # active
    curses.init_pair(5,  curses.COLOR_GREEN,   -1)                   # bar
    curses.init_pair(6,  curses.COLOR_YELLOW,  -1)                   # hint
    curses.init_pair(7,  curses.COLOR_MAGENTA, -1)                   # scope
    curses.init_pair(8,  curses.COLOR_BLACK,   curses.COLOR_YELLOW)  # cursor
    curses.init_pair(9,  curses.COLOR_RED,     -1)                   # filter on
    curses.init_pair(10, curses.COLOR_BLACK,   curses.COLOR_RED)     # kick
    curses.init_pair(11, curses.COLOR_BLACK,   curses.COLOR_YELLOW)  # snare
    curses.init_pair(12, curses.COLOR_BLACK,   curses.COLOR_CYAN)    # hihat
    curses.init_pair(13, curses.COLOR_BLACK,   curses.COLOR_MAGENTA) # top
    curses.init_pair(14, curses.COLOR_WHITE,   curses.COLOR_BLACK)   # step off+cursor
    curses.init_pair(15, curses.COLOR_BLACK,   curses.COLOR_BLUE)    # clap
    curses.init_pair(16, curses.COLOR_BLACK,   curses.COLOR_WHITE)   # tom/cym

    stream = open_audio_stream()

    last_note    = midi_to_name(60)
    held_note    = None      # tracks which note key is "down"
    last_note_t  = 0.0
    NOTE_TIMEOUT = 0.22      # release after 220ms with no key repeat
    HELP_TIMEOUT = 0.20
    help_until   = 0.0
    seq_cursor_v = 0
    seq_cursor_s = 0
    focus = "synth"
    settings_open = False
    settings_page = 0
    settings_cursor = 0
    drum_clear_confirm = None
    drum_notice = ""
    drum_notice_until = 0.0

    with audio_lock:
        refresh_audio_devices_locked()

    with midi_lock:
        refresh_midi_devices_locked()

    try:
        while True:
            ch = stdscr.getch()
            now = time.time()

            if ch == ord('H'):
                help_until = now + HELP_TIMEOUT

            if drum_notice_until and now >= drum_notice_until:
                drum_notice = ""
                drum_notice_until = 0.0
                drum_clear_confirm = None

            # auto-release: curses has no keyup, so we release after timeout
            if held_note is not None and (now - last_note_t) > NOTE_TIMEOUT:
                with synth_lock:
                    synth["key_note_on"] = False
                    synth["key_offset"] = None
                    sync_pitch_locked()
                held_note = None

            if settings_open:
                row_count = settings_row_count(settings_page)
                if ch == ord('q'):
                    break
                elif ch in (27, ord('S')):
                    settings_open = False
                elif ch in (ord(' '), ord('\n'), 10, 13):
                    if settings_page == 1:
                        pass
                    elif settings_page == 2:
                        audio_reopen = False
                        with audio_lock:
                            if settings_cursor == 3:
                                refresh_audio_devices_locked()
                                audio_reopen = True
                        if audio_reopen:
                            close_audio_stream(stream)
                            stream = open_audio_stream()
                    elif settings_page == 3:
                        if settings_cursor == 1:
                            with midi_lock:
                                midi["enabled"] = not midi["enabled"]
                            reopen_midi_input()
                        elif settings_cursor == 3:
                            with midi_lock:
                                refresh_midi_devices_locked()
                            reopen_midi_input()
                    elif settings_page == 4:
                        with midi_lock:
                            if settings_cursor == 2:
                                midi["learn_mode"] = "off" if midi["learn_mode"] == "note_src" else "note_src"
                            elif settings_cursor == 4:
                                midi["note_map"][int(midi["note_edit_in"])] = int(midi["note_edit_out"])
                                midi["status"] = f"Mapped {midi_to_name(midi['note_edit_in'])} to {midi_to_name(midi['note_edit_out'])}"
                            elif settings_cursor == 5:
                                midi["note_map"].pop(int(midi["note_edit_in"]), None)
                                midi["status"] = f"Cleared map for {midi_to_name(midi['note_edit_in'])}"
                    elif settings_page == 5:
                        with midi_lock:
                            if settings_cursor == 2:
                                midi["learn_mode"] = "off" if midi["learn_mode"] == "bind" else "bind"
                            elif settings_cursor == 4:
                                target_id, _, kind = selected_midi_bind_target_locked()
                                if kind == "cc":
                                    midi["cc_bindings"][target_id] = None
                                else:
                                    midi["note_bindings"][target_id] = None
                                midi["status"] = f"Cleared {MIDI_BIND_LABELS[target_id]}"
                elif ch == curses.KEY_UP:
                    settings_cursor = (settings_cursor - 1) % row_count
                elif ch == curses.KEY_DOWN:
                    settings_cursor = (settings_cursor + 1) % row_count
                elif ch in (curses.KEY_LEFT, curses.KEY_RIGHT):
                    delta = -1 if ch == curses.KEY_LEFT else 1
                    if settings_cursor == 0:
                        settings_page = (settings_page + delta) % len(SETTINGS_PAGE_NAMES)
                        settings_cursor = min(settings_cursor, settings_row_count(settings_page) - 1)
                    elif settings_page == 0:
                        if settings_cursor == 1:
                            with ui_lock:
                                ui_state["theme"] = (ui_state["theme"] + delta) % len(THEME_NAMES)
                        elif settings_cursor == 2:
                            with ui_lock:
                                ui_state["visual_mode"] = (ui_state["visual_mode"] + delta) % len(VISUAL_MODES)
                        elif settings_cursor == 3:
                            with ui_lock:
                                ui_state["scope_show_drums"] = not ui_state["scope_show_drums"]
                        elif settings_cursor == 4:
                            with drum_lock:
                                drum["bank"] = (drum["bank"] + delta) % len(DRUM_BANKS)
                        elif settings_cursor == 5:
                            with synth_lock:
                                synth["voices"] = int(clamp(synth["voices"] + delta, 1, MAX_VOICES))
                        elif settings_cursor == 6:
                            with synth_lock:
                                synth["active_osc"] = (synth["active_osc"] + delta) % 2
                        elif settings_cursor == 7:
                            with synth_lock:
                                active_osc_locked()["waveform"] = (active_osc_locked()["waveform"] + delta) % len(WAVEFORMS)
                        elif settings_cursor == 8:
                            with synth_lock:
                                active_osc_locked()["level"] = clamp(active_osc_locked()["level"] + delta * 0.05, 0.0, 1.0)
                        elif settings_cursor == 9:
                            with synth_lock:
                                active_osc_locked()["octave"] = int(clamp(active_osc_locked()["octave"] + delta, -2, 2))
                        elif settings_cursor == 10:
                            with synth_lock:
                                active_osc_locked()["detune_cents"] = clamp(active_osc_locked()["detune_cents"] + delta * 2.0, -24.0, 24.0)
                        elif settings_cursor == 11:
                            with synth_lock:
                                synth["fx_warmth"] = clamp(synth["fx_warmth"] + delta * 0.05, 0.0, 1.0)
                        elif settings_cursor == 12:
                            with synth_lock:
                                synth["fx_air"] = clamp(synth["fx_air"] + delta * 0.05, 0.0, 1.0)
                        elif settings_cursor == 13:
                            with synth_lock:
                                synth["fx_reverb"] = clamp(synth["fx_reverb"] + delta * 0.05, 0.0, 1.0)
                    elif settings_page == 1:
                        with ui_lock:
                            if settings_cursor == 1:
                                ui_state["camera_style"] = (ui_state["camera_style"] + delta) % len(CAMERA_REACTIVE_STYLES)
                            elif settings_cursor == 2:
                                ui_state["camera_reactivity"] = clamp(ui_state["camera_reactivity"] + delta * 0.05, 0.0, 1.0)
                    elif settings_page == 2:
                        audio_reopen = False
                        with audio_lock:
                            if settings_cursor == 1 and audio["outputs"]:
                                audio["output_index"] = (audio["output_index"] + delta) % len(audio["outputs"])
                                audio["output_name"] = audio["outputs"][audio["output_index"]]["name"]
                                audio_reopen = True
                            elif settings_cursor == 2 and audio["inputs"]:
                                audio["input_index"] = (audio["input_index"] + delta) % len(audio["inputs"])
                                audio["input_name"] = audio["inputs"][audio["input_index"]]["name"]
                                audio["status"] = short_label(f"Input selected: {audio['input_name']}", 42)
                            elif settings_cursor == 3:
                                refresh_audio_devices_locked()
                                audio_reopen = True
                        if audio_reopen:
                            close_audio_stream(stream)
                            stream = open_audio_stream()
                    elif settings_page == 3:
                        midi_reopen = False
                        clear_notes = False
                        with midi_lock:
                            if settings_cursor == 1:
                                midi["enabled"] = not midi["enabled"]
                                midi_reopen = True
                            elif settings_cursor == 2:
                                refresh_midi_devices_locked()
                                if midi["devices"]:
                                    midi["device_index"] = (midi["device_index"] + delta) % len(midi["devices"])
                                    midi["device_name"] = midi["devices"][midi["device_index"]]
                                    midi_reopen = True
                            elif settings_cursor == 3:
                                refresh_midi_devices_locked()
                                midi_reopen = True
                            elif settings_cursor == 4:
                                midi["channel"] = -1 if delta < 0 and midi["channel"] == 0 else int(clamp(midi["channel"] + delta, -1, 15))
                            elif settings_cursor == 5:
                                midi["note_input"] = not midi["note_input"]
                                clear_notes = not midi["note_input"]
                            elif settings_cursor == 6:
                                midi["pad_input"] = not midi["pad_input"]
                        if clear_notes:
                            clear_midi_note_state()
                        if midi_reopen:
                            reopen_midi_input()
                    elif settings_page == 4:
                        with midi_lock:
                            if settings_cursor == 1:
                                midi["note_edit_in"] = int(clamp(midi["note_edit_in"] + delta, 0, 127))
                            elif settings_cursor == 2:
                                midi["learn_mode"] = "off" if midi["learn_mode"] == "note_src" else "note_src"
                            elif settings_cursor == 3:
                                midi["note_edit_out"] = int(clamp(midi["note_edit_out"] + delta, 0, 127))
                            elif settings_cursor == 4:
                                midi["note_map"][int(midi["note_edit_in"])] = int(midi["note_edit_out"])
                                midi["status"] = f"Mapped {midi_to_name(midi['note_edit_in'])} to {midi_to_name(midi['note_edit_out'])}"
                            elif settings_cursor == 5:
                                midi["note_map"].pop(int(midi["note_edit_in"]), None)
                                midi["status"] = f"Cleared map for {midi_to_name(midi['note_edit_in'])}"
                    elif settings_page == 5:
                        with midi_lock:
                            if settings_cursor == 1:
                                midi["learn_target_index"] = (midi["learn_target_index"] + delta) % len(MIDI_BIND_TARGETS)
                            elif settings_cursor == 2:
                                midi["learn_mode"] = "off" if midi["learn_mode"] == "bind" else "bind"
                            elif settings_cursor == 4:
                                target_id, _, kind = selected_midi_bind_target_locked()
                                if kind == "cc":
                                    midi["cc_bindings"][target_id] = None
                                else:
                                    midi["note_bindings"][target_id] = None
                                midi["status"] = f"Cleared {MIDI_BIND_LABELS[target_id]}"
            else:
                if ch == ord('\t'):
                    focus = "seq" if focus=="synth" else "synth"
                    with synth_lock:
                        synth["key_note_on"] = False
                        synth["key_offset"] = None
                        sync_pitch_locked()
                    held_note = None
                elif ch == ord('S'):
                    settings_open = True
                elif focus == "synth":
                    if ch in KEYBOARD_OFFSETS:
                        with synth_lock:
                            synth["key_offset"] = KEYBOARD_OFFSETS[ch]
                            synth["key_note_on"] = True
                            sync_pitch_locked()
                            play_midi = current_play_midi_locked()
                        last_note   = midi_to_name(play_midi)
                        held_note   = ch
                        last_note_t = now
                    elif ch == ord(' '):
                        with synth_lock:
                            synth["key_note_on"] = False
                            synth["key_offset"] = None
                            sync_pitch_locked()
                        held_note = None
                    elif ch == ord('q'):
                        break
                    elif ch == ord('1'):
                        with synth_lock: synth["active_osc"] = 0
                    elif ch == ord('2'):
                        with synth_lock: synth["active_osc"] = 1
                    elif ch == ord('z'):
                        with synth_lock: active_osc_locked()["waveform"]=(active_osc_locked()["waveform"]-1)%4
                    elif ch == ord('x'):
                        with synth_lock: active_osc_locked()["waveform"]=(active_osc_locked()["waveform"]+1)%4
                    elif ch == ord('m'):
                        with synth_lock: synth["gate_mode"]=1-synth["gate_mode"]
                    elif ch == ord('p'):
                        with synth_lock: synth["filter_on"]=not synth["filter_on"]
                    elif ch == ord('l'):
                        with synth_lock: synth["lfo_wave"]=(synth["lfo_wave"]+1)%3
                    elif ch == ord('o'):
                        with synth_lock: synth["lfo_target"]=(synth["lfo_target"]+1)%3
                    elif ch == ord('R'):
                        if loop["recording"]:
                            stop_loop_recording()
                        else:
                            with loop_lock:
                                clear_loop_locked()
                                loop["recording"] = True
                    elif ch == ord('T'):
                        with loop_lock:
                            if loop["recording"] and loop["overdub"]:
                                loop["recording"] = False
                                loop["overdub"] = False
                            elif loop["has_audio"]:
                                push_loop_undo_locked()
                                loop["recording"] = True
                                loop["playing"] = True
                                loop["overdub"] = True
                                loop["write_pos"] = loop["read_pos"]
                    elif ch == ord('Y'):
                        undo_last_overdub()
                    elif ch == ord('P'):
                        with loop_lock:
                            if loop["has_audio"]:
                                loop["playing"] = not loop["playing"]
                    elif ch == ord('U'):
                        with loop_lock:
                            clear_loop_locked()
                    elif ch == ord('G'):
                        if global_rec["recording"]:
                            stop_global_recording()
                        else:
                            start_global_recording()
                    elif ch == curses.KEY_UP:
                        with synth_lock: synth["volume"]=clamp(synth["volume"]+0.05,0,1)
                    elif ch == curses.KEY_DOWN:
                        with synth_lock: synth["volume"]=clamp(synth["volume"]-0.05,0,1)
                    elif ch == curses.KEY_RIGHT:
                        with synth_lock:
                            synth["base_midi"] = int(clamp(synth["base_midi"] + 1, 36, 84))
                            sync_pitch_locked()
                    elif ch == curses.KEY_LEFT:
                        with synth_lock:
                            synth["base_midi"] = int(clamp(synth["base_midi"] - 1, 36, 84))
                            sync_pitch_locked()
                    elif ch == ord('['):
                        with synth_lock: synth["attack"]=clamp(synth["attack"]-0.01,0.001,2.0)
                    elif ch == ord(']'):
                        with synth_lock: synth["attack"]=clamp(synth["attack"]+0.01,0.001,2.0)
                    elif ch == ord('{'):
                        with synth_lock: synth["release"]=clamp(synth["release"]-0.02,0.01,4.0)
                    elif ch == ord('}'):
                        with synth_lock: synth["release"]=clamp(synth["release"]+0.02,0.01,4.0)
                    elif ch == ord(','):
                        with synth_lock: synth["lfo_rate"]=clamp(synth["lfo_rate"]-0.1,0.1,20.0)
                    elif ch == ord('.'):
                        with synth_lock: synth["lfo_rate"]=clamp(synth["lfo_rate"]+0.1,0.1,20.0)
                    elif ch in (ord(';'), ord(':')):
                        with synth_lock: synth["lfo_depth"]=clamp(synth["lfo_depth"]-0.02,0.0,1.0)
                    elif ch in (ord("'"), ord('"')):
                        with synth_lock: synth["lfo_depth"]=clamp(synth["lfo_depth"]+0.02,0.0,1.0)
                    elif ch == ord('-'):
                        with synth_lock: synth["cutoff"]=clamp(synth["cutoff"]-0.02,0.0,1.0)
                    elif ch == ord('='):
                        with synth_lock: synth["cutoff"]=clamp(synth["cutoff"]+0.02,0.0,1.0)
                    elif ch == ord('_'):
                        with synth_lock: synth["resonance"]=clamp(synth["resonance"]-0.02,0.0,0.99)
                    elif ch == ord('+'):
                        with synth_lock: synth["resonance"]=clamp(synth["resonance"]+0.02,0.0,0.99)
                    elif ch == ord('D'):
                        with synth_lock: synth["fx_drive"] = clamp(synth["fx_drive"] - 0.05, 0.0, 1.0)
                    elif ch == ord('F'):
                        with synth_lock: synth["fx_drive"] = clamp(synth["fx_drive"] + 0.05, 0.0, 1.0)
                    elif ch == ord('J'):
                        with synth_lock: synth["fx_delay_mix"] = clamp(synth["fx_delay_mix"] - 0.05, 0.0, 1.0)
                    elif ch == ord('K'):
                        with synth_lock: synth["fx_delay_mix"] = clamp(synth["fx_delay_mix"] + 0.05, 0.0, 1.0)
                    elif ch == ord('N'):
                        with synth_lock: synth["fx_delay_feedback"] = clamp(synth["fx_delay_feedback"] - 0.05, 0.0, 0.95)
                    elif ch == ord('M'):
                        with synth_lock: synth["fx_delay_feedback"] = clamp(synth["fx_delay_feedback"] + 0.05, 0.0, 0.95)
                    elif ch == ord('V'):
                        with synth_lock: synth["fx_delay_time"] = clamp(synth["fx_delay_time"] - 0.05, 0.0, 1.0)
                    elif ch == ord('B'):
                        with synth_lock: synth["fx_delay_time"] = clamp(synth["fx_delay_time"] + 0.05, 0.0, 1.0)

                else:  # focus == "seq"
                    if ch == ord('q'): break
                    elif ch == curses.KEY_UP:
                        drum_clear_confirm = None
                        seq_cursor_v = (seq_cursor_v - 1) % NUM_DRUM_VOICES
                    elif ch == curses.KEY_DOWN:
                        drum_clear_confirm = None
                        seq_cursor_v = (seq_cursor_v + 1) % NUM_DRUM_VOICES
                    elif ch == curses.KEY_LEFT:
                        drum_clear_confirm = None
                        seq_cursor_s = (seq_cursor_s - 1) % NUM_STEPS
                    elif ch == curses.KEY_RIGHT:
                        drum_clear_confirm = None
                        seq_cursor_s = (seq_cursor_s + 1) % NUM_STEPS
                    elif ch == ord(' '):
                        drum_clear_confirm = None
                        with drum_lock:
                            drum["steps"][seq_cursor_v][seq_cursor_s] = \
                                not drum["steps"][seq_cursor_v][seq_cursor_s]
                    elif ch == ord('\n') or ch == ord('r'):
                        drum_clear_confirm = None
                        with drum_lock: drum["running"] = not drum["running"]
                    elif ch == ord('c'):
                        if drum_clear_confirm == ("row", seq_cursor_v) and now < drum_notice_until:
                            with drum_lock:
                                drum["steps"][seq_cursor_v] = [False]*NUM_STEPS
                            drum_notice = f"Cleared {DRUM_NAMES[seq_cursor_v].strip()} row"
                            drum_notice_until = now + 1.2
                            drum_clear_confirm = None
                        else:
                            drum_clear_confirm = ("row", seq_cursor_v)
                            drum_notice = f"Press c again to clear {DRUM_NAMES[seq_cursor_v].strip()} row"
                            drum_notice_until = now + 1.2
                    elif ch == ord('X'):
                        if drum_clear_confirm == ("all", None) and now < drum_notice_until:
                            with drum_lock:
                                drum["steps"] = [[False]*NUM_STEPS for _ in range(NUM_DRUM_VOICES)]
                            drum_notice = "Cleared all drum steps"
                            drum_notice_until = now + 1.2
                            drum_clear_confirm = None
                        else:
                            drum_clear_confirm = ("all", None)
                            drum_notice = "Press Shift+X again to clear all drum steps"
                            drum_notice_until = now + 1.2
                    elif ch == ord(','):
                        drum_clear_confirm = None
                        with drum_lock: drum["bpm"] = clamp(drum["bpm"]-1, 40, 300)
                    elif ch == ord('.'):
                        drum_clear_confirm = None
                        with drum_lock: drum["bpm"] = clamp(drum["bpm"]+1, 40, 300)
                    elif ch == ord('<'):
                        drum_clear_confirm = None
                        with drum_lock: drum["bpm"] = clamp(drum["bpm"]-5, 40, 300)
                    elif ch == ord('>'):
                        drum_clear_confirm = None
                        with drum_lock: drum["bpm"] = clamp(drum["bpm"]+5, 40, 300)
                    elif ch == ord('-'):
                        drum_clear_confirm = None
                        with drum_lock: drum["vol"][seq_cursor_v]=clamp(drum["vol"][seq_cursor_v]-0.05,0,1)
                    elif ch == ord('='):
                        drum_clear_confirm = None
                        with drum_lock: drum["vol"][seq_cursor_v]=clamp(drum["vol"][seq_cursor_v]+0.05,0,1)
                    elif ch == ord('1'):
                        drum_clear_confirm = None
                        with drum_lock:
                            apply_drum_pattern_locked(0)
                    elif ch == ord('2'):
                        drum_clear_confirm = None
                        with drum_lock:
                            apply_drum_pattern_locked(1)

            # ── snapshot ───────────────────────────
            with synth_lock:
                vol=synth["volume"]; freq=current_play_freq_locked()
                base_midi=synth["base_midi"]; voices=synth["voices"]
                atk=synth["attack"]; rel=synth["release"]; env=synth["env"]
                note=note_active_locked(); gate=synth["gate_mode"]
                lfo_wf=synth["lfo_wave"]; lfo_rate=synth["lfo_rate"]
                lfo_dep=synth["lfo_depth"]; lfo_tgt=synth["lfo_target"]
                filt_on=synth["filter_on"]; cutoff=synth["cutoff"]
                res=synth["resonance"]; lfo_ph=synth["lfo_phase"]
                fx_drive=synth["fx_drive"]; fx_mix=synth["fx_delay_mix"]
                fx_feedback=synth["fx_delay_feedback"]; fx_time=synth["fx_delay_time"]
                fx_warmth=synth["fx_warmth"]; fx_air=synth["fx_air"]; fx_reverb=synth["fx_reverb"]
                oscillators=[{
                    "waveform": osc["waveform"],
                    "level": osc["level"],
                    "octave": osc["octave"],
                    "detune_cents": osc["detune_cents"],
                } for osc in synth["oscillators"]]
                active_osc=synth["active_osc"]
                xruns=synth["xruns"]
                play_midi=current_play_midi_locked()

            with loop_lock:
                loop_recording=loop["recording"]
                loop_playing=loop["playing"]
                loop_overdub=loop["overdub"]
                loop_has_audio=loop["has_audio"]
                loop_length=loop["length"]
                loop_undo_depth=len(loop["undo_stack"])

            with global_rec_lock:
                global_recording=global_rec["recording"]
                global_last_path=global_rec["last_path"]
                global_last_error=global_rec["last_error"]

            with ui_lock:
                theme=ui_state["theme"]
                visual_mode=ui_state["visual_mode"]
                camera_style=ui_state["camera_style"]
                camera_reactivity=ui_state["camera_reactivity"]
                scope_show_drums=ui_state["scope_show_drums"]

            with audio_lock:
                audio_output_name = audio["output_name"]
                audio_input_name = audio["input_name"]
                audio_output_count = len(audio["outputs"])
                audio_input_count = len(audio["inputs"])
                audio_status = audio["status"]

            with reactive_lock:
                rx_master = reactive_state["master"]
                rx_synth = reactive_state["synth"]
                rx_drums = reactive_state["drums"]
                rx_kick = reactive_state["kick"]

            with midi_lock:
                midi_enabled = midi["enabled"]
                midi_device_name = midi["device_name"]
                midi_device_count = len(midi["devices"])
                midi_status = midi["status"]
                midi_last_message = midi["last_message"]
                midi_channel = midi["channel"]
                midi_note_input = midi["note_input"]
                midi_pad_input = midi["pad_input"]
                midi_learn_mode = midi["learn_mode"]
                midi_note_edit_in = midi["note_edit_in"]
                midi_note_edit_out = midi["note_edit_out"]
                midi_note_map_count = len(midi["note_map"])
                midi_note_map_current = midi["note_map"].get(midi_note_edit_in)
                midi_target_label, midi_target_kind, midi_target_value = selected_midi_binding_locked()

            with drum_lock:
                d_steps   = [row[:] for row in drum["steps"]]
                d_running = drum["running"]
                d_bpm     = drum["bpm"]
                d_bank_name = current_drum_bank_locked()["name"]
                d_cur     = drum["cur_step"]
                d_vol     = drum["vol"][:]

            with scope_lock:
                snap = list(scope_buf)

            h, w = stdscr.getmaxyx()
            stdscr.erase()

            C  = [curses.color_pair(i) for i in range(17)]
            B  = curses.A_BOLD
            DIM= curses.A_DIM
            scope_attr = [C[7], C[5], C[6]][theme % 3]

            # ══════════════════════════════════════
            #  LAYOUT:
            #  col 0..39  : synth controls
            #  col 40..w  : scope (top half) + drums (bottom half)
            # ══════════════════════════════════════
            CTRL_W   = 40
            scope_x  = CTRL_W
            DRUM_H   = 9
            drum_top = max(h - DRUM_H, 2)
            show_help = (now < help_until) and not settings_open
            base_note_name = midi_to_name(base_midi)
            play_note_name = midi_to_name(play_midi)
            delay_ms = int(80 + fx_time * 720)
            loop_secs = loop_length / SAMPLE_RATE if loop_length else 0.0

            # ── header ─────────────────────────────
            foc_ind = " [SYNTH] " if focus=="synth" else " [DRUM]  "
            safe_addstr(stdscr, 0, 0, "  ♪  MurSynth  ♪  ".center(w), C[1]|B)
            btn_attr = (scope_attr|B) if settings_open else C[3]
            help_attr = (scope_attr|B) if show_help else C[3]
            safe_addstr(stdscr, 0, max(2, w-31), "[SET S]", btn_attr)
            safe_addstr(stdscr, 0, max(10, w-22), "[HELP H]", help_attr)
            safe_addstr(stdscr, 0, max(19, w-11), foc_ind, C[8]|B)

            # ── OSC ────────────────────────────────
            safe_addstr(stdscr,  2, 2, "─ OSC ─────────────────────", C[2])
            safe_addstr(stdscr,  3, 4, "EDIT", C[2])
            safe_addstr(stdscr,  3, 10, " OSC1 ", (C[4]|B) if active_osc == 0 else C[3])
            safe_addstr(stdscr,  3, 17, " OSC2 ", (C[4]|B) if active_osc == 1 else C[3])
            safe_addstr(stdscr,  3, 25, "1/2 z/x", C[6])
            for osc_idx, osc in enumerate(oscillators):
                row = 4 + osc_idx
                sel = (C[4]|B) if osc_idx == active_osc else C[3]
                oct_lbl = f"{2 ** osc['octave']}x" if osc["octave"] >= 0 else f"1/{2 ** abs(osc['octave'])}x"
                safe_addstr(stdscr, row, 4, f"O{osc_idx + 1}", C[2])
                safe_addstr(stdscr, row, 8, f"{WAVEFORMS[osc['waveform']]} {int(osc['level']*100):3d}% {oct_lbl:>4} {osc['detune_cents']:+5.1f}c", sel)
            gate_lbl=" FREE " if gate==1 else " GATE "
            safe_addstr(stdscr,6,4,"MODE",C[2])
            safe_addstr(stdscr,6,10,gate_lbl,(C[4]|B) if gate==1 else C[3])
            safe_addstr(stdscr,6,18,"m",C[6])
            bw=16
            safe_addstr(stdscr,7,4,"VOL ",C[2])
            safe_addstr(stdscr,7,8,hbar(vol,bw),C[5])
            safe_addstr(stdscr,7,8+bw+1,f"{int(vol*100):3d}%",C[3])
            safe_addstr(stdscr,7,8+bw+5,"↑↓",C[6])
            safe_addstr(stdscr,8,4,"BASE",C[2])
            safe_addstr(stdscr,8,9,f"{base_note_name:>3} {midi_to_freq(base_midi):7.2f}Hz",C[3]|B)
            safe_addstr(stdscr,8,25,"←→",C[6])
            safe_addstr(stdscr,9,4,"PLAY",C[2])
            safe_addstr(stdscr,9,9,f"{play_note_name:>3} {freq:7.2f}Hz",(C[4]|B) if (note or gate==1) else C[3])
            safe_addstr(stdscr,9,25,f"VOI {int(voices)}",C[6])
            loop_state = "REC" if loop_recording and not loop_overdub else ("DUB" if loop_overdub else ("PLY" if loop_playing else ("HLD" if loop_has_audio else "OFF")))
            safe_addstr(stdscr,10,4,f"LOOP {loop_state:>3} {loop_secs:4.1f}s", (C[4]|B) if loop_recording or loop_playing else C[3])
            safe_addstr(stdscr,10,24,f"UND {loop_undo_depth:02d}", C[6])
            safe_addstr(stdscr,10,34,f"XR {xruns:02d}", C[6])
            grec_lbl = "GREC ON" if global_recording else "GREC OFF"
            safe_addstr(stdscr,11,4,grec_lbl, (C[4]|B) if global_recording else C[3])
            grec_msg = global_last_error if global_last_error else (os.path.basename(global_last_path) if global_last_path else "")
            safe_addstr(stdscr,11,14,grec_msg[:24], C[6]|DIM)

            # ── ENV ────────────────────────────────
            safe_addstr(stdscr,12,2,"─ ENV ─────────────────────",C[2])
            safe_addstr(stdscr,13,4,"ATK",C[2]); safe_addstr(stdscr,13,8,f"{atk:.3f}s",C[3]); safe_addstr(stdscr,13,17,"[ ]",C[6])
            safe_addstr(stdscr,13,22,"REL",C[2]); safe_addstr(stdscr,13,26,f"{rel:.3f}s",C[3]); safe_addstr(stdscr,13,35,"{ }",C[6])
            safe_addstr(stdscr,14,4,"ENV",C[2]); safe_addstr(stdscr,14,8,hbar(env,bw,"▓","░"),C[5] if (note or gate==1) else C[3])

            # ── LFO ────────────────────────────────
            safe_addstr(stdscr,16,2,"─ LFO ─────────────────────",C[2])
            safe_addstr(stdscr,17,4,"WAVE",C[2])
            x=10
            for i,nm in enumerate(LFO_WAVES):
                safe_addstr(stdscr,17,x,f" {nm} ",(C[4]|B) if i==lfo_wf else C[3]); x+=6
            safe_addstr(stdscr,17,x+1,"l",C[6])
            safe_addstr(stdscr,17,25,"TRGT",C[2])
            x=30
            for i,nm in enumerate(LFO_TARGETS):
                safe_addstr(stdscr,17,x,f" {nm} ",(C[4]|B) if i==lfo_tgt else C[3]); x+=7
            safe_addstr(stdscr,17,min(x+1, 37),"o",C[6])
            safe_addstr(stdscr,18,4,"RATE",C[2]); safe_addstr(stdscr,18,9,f"{lfo_rate:5.2f}Hz",C[3]); safe_addstr(stdscr,18,18,",/.",C[6])
            safe_addstr(stdscr,18,24,"DEPT",C[2]); safe_addstr(stdscr,18,29,hbar(lfo_dep,8),C[5]); safe_addstr(stdscr,18,38,";/'",C[6])
            lx=int((lfo_ph%1.0)*12)
            safe_addstr(stdscr,19,9," "*lx+"◆"+" "*(11-lx),scope_attr)

            # ── FILTER ─────────────────────────────
            safe_addstr(stdscr,21,2,"─ FILTER ───────────────────",C[2])
            safe_addstr(stdscr,22,4,"LPF ",C[2])
            safe_addstr(stdscr,22,8," ON  " if filt_on else " OFF ",(C[9]|B) if filt_on else C[3])
            safe_addstr(stdscr,22,15,"p",C[6])
            cHz=int(20*(1000**cutoff))
            safe_addstr(stdscr,22,19,"CUT",C[2]); safe_addstr(stdscr,22,23,hbar(cutoff,8),C[5] if filt_on else C[3]); safe_addstr(stdscr,22,32,f"{cHz:5d}",C[3]); safe_addstr(stdscr,22,38,"-/=",C[6])
            safe_addstr(stdscr,23,4,"RES ",C[2]); safe_addstr(stdscr,23,8,hbar(res,12),C[5] if filt_on else C[3]); safe_addstr(stdscr,23,21,f"{int(res*100):3d}%",C[3]); safe_addstr(stdscr,23,30,"_/+",C[6])

            # ── FX ─────────────────────────────────
            safe_addstr(stdscr,25,2,"─ FX ───────────────────────",C[2])
            safe_addstr(stdscr,26,4,"DRV ",C[2]); safe_addstr(stdscr,26,9,hbar(fx_drive,8),C[5]); safe_addstr(stdscr,26,18,"D/F",C[6])
            safe_addstr(stdscr,26,24,"TIME",C[2]); safe_addstr(stdscr,26,29,f"{delay_ms:3d}ms",C[3]); safe_addstr(stdscr,26,36,"V/B",C[6])
            safe_addstr(stdscr,27,4,"DLY ",C[2]); safe_addstr(stdscr,27,9,hbar(fx_mix,8),C[5]); safe_addstr(stdscr,27,18,"J/K",C[6])
            safe_addstr(stdscr,27,24,"FBK ",C[2]); safe_addstr(stdscr,27,29,hbar(fx_feedback,6),C[5]); safe_addstr(stdscr,27,36,"N/M",C[6])
            safe_addstr(stdscr,28,4,"WRM ",C[2]); safe_addstr(stdscr,28,8,hbar(fx_warmth,4),C[5])
            safe_addstr(stdscr,28,16,"AIR ",C[2]); safe_addstr(stdscr,28,20,hbar(fx_air,4),C[5])
            safe_addstr(stdscr,28,28,"RVB ",C[2]); safe_addstr(stdscr,28,32,hbar(fx_reverb,4),C[5])

            # ── KEYS ───────────────────────────────
            safe_addstr(stdscr,29,2,"KEYS",C[2])
            safe_addstr(stdscr,29,8,"a w s e d f t g y h u j k",C[3])
            key_status = f" ♪ {last_note} " if note else (" ♪ FREE " if gate==1 else f" ROOT {base_note_name} ")
            safe_addstr(stdscr,30,2,key_status,(C[4]|B) if (note or gate==1) else C[3])
            safe_addstr(stdscr,30,16,"R rec  T dub  Y undo  P play  U clear  G wav",C[6]|DIM)

            # ══════════════════════════════════════
            #  SCOPE  (top-right)
            # ══════════════════════════════════════
            sw        = max(4, w - scope_x - 1)
            scope_top = 1
            scope_bot = drum_top - 1
            sh        = max(1, scope_bot - scope_top)

            visual_title = "CAM" if visual_mode == 1 else "SCOPE"
            safe_addstr(stdscr, scope_top, scope_x, visual_title + " " + "─"*(max(0, sw-len(visual_title)-1)), C[2])
            if visual_mode == 1:
                lines = render_camera_visual(sw, sh-1, camera_style, camera_reactivity, C, scope_attr, B, DIM)
            else:
                stop_camera_stream()
                lines = render_scope(snap, sw, sh-1)
            for row, line in enumerate(lines):
                draw_visual_line(stdscr, scope_top+1+row, scope_x, line, scope_attr)

            scope_lbl = (f"CAM {camera_style:02d}" if visual_mode == 1 else ("MIX" if scope_show_drums else "SYNTH"))
            safe_addstr(stdscr, scope_top, max(scope_x + 7, w - 15), f"[{THEME_NAMES[theme]} {scope_lbl}]", scope_attr|B)

            # ══════════════════════════════════════
            #  DRUM SEQUENCER  (bottom strip)
            # ══════════════════════════════════════
            run_col  = C[4]|B if d_running else C[3]
            run_lbl  = " ▶ RUN " if d_running else " ■ STP "
            bpm_str  = f"BPM:{int(d_bpm):3d}"

            # section header
            seq_focus_attr = C[8]|B if focus=="seq" else C[2]
            safe_addstr(stdscr, drum_top, 0,
                        "─ DRUMS " + "─"*(w-9), C[2])
            safe_addstr(stdscr, drum_top, 2, "─ DRUMS ", seq_focus_attr)
            safe_addstr(stdscr, drum_top, 10, run_lbl, run_col)
            safe_addstr(stdscr, drum_top, 18, bpm_str, C[3])
            safe_addstr(stdscr, drum_top, 27, ",/. bpm", C[6])
            safe_addstr(stdscr, drum_top, 36, "r=start/stop", C[6])
            if w > 60:
                safe_addstr(stdscr, drum_top, 49, f"set:{d_bank_name}", C[6])
            if drum_notice:
                safe_addstr(stdscr, drum_top, max(58, w - len(drum_notice) - 2), drum_notice[:max(0, w-60)], C[6]|B)

            # step numbers header
            step_x0 = 6
            step_cell_w = 2
            for s in range(NUM_STEPS):
                sx = step_x0 + s*step_cell_w
                if s % 4 == 0:
                    safe_addstr(stdscr, drum_top+1, sx, f"{(s+1)%100:02d}", C[2])
                else:
                    safe_addstr(stdscr, drum_top+1, sx, "· ", C[2]|DIM)

            # playhead indicator
            ph_x = step_x0 + d_cur*step_cell_w
            if d_running:
                safe_addstr(stdscr, drum_top+1, ph_x, "▼", C[4]|B)

            # voice rows
            VOICE_COLORS = [10, 11, 15, 12, 16, 13]
            for v in range(NUM_DRUM_VOICES):
                row = drum_top + 2 + v
                # voice label + volume
                vlbl = DRUM_NAMES[v]
                is_selected = (focus=="seq" and seq_cursor_v==v)
                lbl_attr = (C[8]|B) if is_selected else C[2]
                safe_addstr(stdscr, row, 0, f"{vlbl}", lbl_attr)
                # small vol indicator
                vv = d_vol[v]
                safe_addstr(stdscr, row, 5, "▕", C[2]|DIM)

                for s in range(NUM_STEPS):
                    sx = step_x0 + s*step_cell_w
                    on = d_steps[v][s]
                    is_cur_step = d_running and s==d_cur
                    is_cursor   = (focus=="seq" and seq_cursor_v==v and seq_cursor_s==s)

                    if is_cursor:
                        ch_str = "■ " if on else "· "
                        attr   = C[8] | B
                    elif on:
                        ch_str = "■ "
                        attr   = (C[VOICE_COLORS[v]] | B) if is_cur_step else C[VOICE_COLORS[v]]
                    else:
                        ch_str = "· "
                        attr   = (C[5]|DIM) if is_cur_step else (C[3]|DIM)

                    safe_addstr(stdscr, row, sx, ch_str, attr)

                # vol bar at end
                vol_x = step_x0 + NUM_STEPS*step_cell_w + 1
                safe_addstr(stdscr, row, vol_x, hbar(vv, 6), C[5] if is_selected else (C[5]|DIM))
                safe_addstr(stdscr, row, vol_x+7, "-/=" if is_selected else "   ", C[6])

            # footer
            if settings_open:
                ftr = " ↑↓ row | ←→ change/page | S/Esc close | H help | q quit "
            elif focus == "synth":
                ftr = " TAB=drums | R/T/Y/P/U loop | G wav | S settings | q quit "
            else:
                ftr = " TAB=synth | ←→↑↓ move | SPC toggle | r run | 1/2 patterns | S settings | q quit "
            safe_addstr(stdscr, h-1, 0, ftr[:w-1], C[1])

            if show_help:
                draw_help_overlay(stdscr, h, w, scope_attr, C, B, DIM)

            if settings_open:
                box_w = min(68, max(38, w - 8))
                box_h = 19 if settings_page == 0 else (15 if settings_page in (1, 2) else 14)
                box_x = max(2, (w - box_w) // 2)
                box_y = max(2, (h - box_h) // 2)
                draw_box(stdscr, box_y, box_x, box_w, box_h, f"SETTINGS {SETTINGS_PAGE_NAMES[settings_page]}", scope_attr|B)
                tab_x = box_x + 2
                for page_idx, page_name in enumerate(SETTINGS_PAGE_NAMES):
                    tab_attr = (C[8]|B) if page_idx == settings_page else C[3]
                    safe_addstr(stdscr, box_y + 1, tab_x, f" {page_name} ", tab_attr)
                    tab_x += len(page_name) + 3
                channel_lbl = "ALL" if midi_channel < 0 else f"CH {midi_channel + 1}"
                current_map_lbl = midi_to_name(midi_note_map_current) if midi_note_map_current is not None else "--"
                rows = [f"Section      {SETTINGS_PAGE_NAMES[settings_page]}"]
                if settings_page == 0:
                    rows.extend([
                        f"Theme        {THEME_NAMES[theme]}",
                        f"Visuals      {VISUAL_MODES[visual_mode]}",
                        f"Scope source {'MIX' if scope_show_drums else 'SYNTH ONLY'}",
                        f"Drum set     {d_bank_name}",
                        f"Voices       {int(voices)}",
                        f"Edit osc     OSC{active_osc + 1}",
                        f"Waveform     {WAVEFORMS[oscillators[active_osc]['waveform']]}",
                        f"Osc level    {int(oscillators[active_osc]['level'] * 100):3d}%",
                        f"Osc octave   {oscillators[active_osc]['octave']:+d}",
                        f"Osc detune   {oscillators[active_osc]['detune_cents']:+5.1f}c",
                        f"FX warmth    {int(fx_warmth * 100):3d}%",
                        f"FX air       {int(fx_air * 100):3d}%",
                        f"FX reverb    {int(fx_reverb * 100):3d}%",
                    ])
                elif settings_page == 1:
                    rows.extend([
                        f"Cam style    {CAMERA_REACTIVE_STYLES[camera_style]}",
                        f"Cam depth    {int(camera_reactivity * 100):3d}%",
                        f"Kick pulse   {int(rx_kick * 100):3d}%",
                        f"Synth glow   {int(rx_synth * 100):3d}%",
                        f"Drum hit     {int(rx_drums * 100):3d}%",
                        f"Master lvl   {int(rx_master * 100):3d}%",
                    ])
                elif settings_page == 2:
                    rows.extend([
                        f"Output dev   {short_label(audio_output_name or 'Default', 42)}",
                        f"Input dev    {short_label(audio_input_name or 'None', 42)}",
                        f"Refresh      {audio_output_count:2d} out / {audio_input_count:2d} in",
                        f"Audio in     Reserved for future input features",
                        f"Audio stat   {short_label(audio_status, 42)}",
                        f"How to use   Select output now, input is stored only",
                    ])
                elif settings_page == 3:
                    rows.extend([
                        f"MIDI input   {'ON ' if midi_enabled else 'OFF'}",
                        f"Device       {short_label(midi_device_name or 'None', 42)}",
                        f"Refresh      {midi_device_count:2d} devices",
                        f"Channel      {channel_lbl}",
                        f"Keys in      {'ON ' if midi_note_input else 'OFF'}",
                        f"Pads in      {'ON ' if midi_pad_input else 'OFF'}",
                        f"Remap next   Go to MIDI MAP / Learn bind",
                        f"Status       {short_label(midi_status, 42)}",
                    ])
                elif settings_page == 4:
                    rows.extend([
                        f"Source note  {midi_to_name(midi_note_edit_in):>3} ({midi_note_edit_in:03d})",
                        f"Learn src    {'ARMED' if midi_learn_mode == 'note_src' else 'OFF'}",
                        f"Dest note    {midi_to_name(midi_note_edit_out):>3} ({midi_note_edit_out:03d})",
                        f"Save map     {midi_to_name(midi_note_edit_in)} → {midi_to_name(midi_note_edit_out)}",
                        f"Clear map    {midi_to_name(midi_note_edit_in)} ({current_map_lbl})",
                        f"Map count    {midi_note_map_count:3d}",
                        f"Last MIDI    {short_label(midi_last_message or '--', 42)}",
                    ])
                else:
                    rows.extend([
                        f"Learn target {midi_target_label}",
                        f"Learn bind   {'ARMED' if midi_learn_mode == 'bind' else 'OFF'}",
                        f"Bound input  {format_binding_value(midi_target_kind, midi_target_value)}",
                        f"Clear bind   {midi_target_label}",
                        f"How to bind  Select target, arm Learn, move knob/hit pad",
                        f"Last MIDI    {short_label(midi_last_message or '--', 42)}",
                    ])
                for idx, row_text in enumerate(rows):
                    attr = (C[8]|B) if idx == settings_cursor else C[3]
                    safe_addstr(stdscr, box_y + 3 + idx, box_x + 2, row_text.ljust(box_w-4), attr)
                footer = "Row 1 switches page. ↑↓ select, ←→ change, Enter/Space run learn/save/clear."
                safe_addstr(stdscr, box_y + box_h - 2, box_x + 2, footer[:box_w-4], C[6])

            stdscr.refresh()
            time.sleep(0.04)

    finally:
        if global_rec["recording"]:
            stop_global_recording()
        stop_camera_stream()
        with midi_lock:
            midi_port = midi["input_port"]
            midi["input_port"] = None
        if midi_port is not None:
            try:
                midi_port.close()
            except Exception:
                pass
        close_audio_stream(stream)

curses.wrapper(draw)
PYEOF

# ── bootstrap venv on first run ──────────
if [ ! -f "$VENV_DIR/bin/python" ]; then
  echo "First run: setting up virtual environment..."
  python3 -m venv "$VENV_DIR"
fi

if ! "$VENV_DIR/bin/python" - <<'PY' >/dev/null 2>&1
import numpy, sounddevice, mido, rtmidi
PY
then
  echo "Installing MurSynth dependencies..."
  "$VENV_DIR/bin/pip" install --quiet numpy sounddevice mido python-rtmidi
  echo "Done! Starting synth..."
  sleep 1
fi

exec "$VENV_DIR/bin/python" "$PY_SCRIPT"
