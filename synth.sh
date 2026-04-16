#!/usr/bin/env bash
# ─────────────────────────────────────────
#  MurSynth — terminal synthesizer + drums
# ─────────────────────────────────────────

VENV_DIR="$HOME/.mursynth-venv"
PY_SCRIPT="$HOME/.mursynth.py"

cat > "$PY_SCRIPT" << 'PYEOF'
import curses, numpy as np, sounddevice as sd
import threading, time, textwrap
from collections import deque

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

KEYBOARD_OFFSETS = {
    ord('a'):0,  ord('w'):1,  ord('s'):2,  ord('e'):3,
    ord('d'):4,  ord('f'):5,  ord('t'):6,  ord('g'):7,
    ord('y'):8,  ord('h'):9,  ord('u'):10, ord('j'):11,
    ord('k'):12,
}
NOTE_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"]
THEME_NAMES = ["MAGENTA", "MINT", "AMBER"]

# ═══════════════════════════════════════════════════════════════════════════════
#  SYNTH STATE
# ═══════════════════════════════════════════════════════════════════════════════
synth = {
    "volume": 0.5,
    "volume_current": 0.5,
    "base_midi": 60, "active_offset": None,
    "attack":  0.01, "release": 0.3, "env": 0.0,
    "gate_mode": 0, "note_on": False,
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
}
loop_lock = threading.Lock()

ui_state = {
    "theme": 0,
    "scope_show_drums": True,
}
ui_lock = threading.Lock()

# ═══════════════════════════════════════════════════════════════════════════════
#  DRUM STATE
# ═══════════════════════════════════════════════════════════════════════════════
DRUM_NAMES  = ["KICK", "SNRE", "HIHT", "TOPP"]
DRUM_COLORS = [3, 4, 5, 6]   # colour pair indices per voice
NUM_STEPS   = 16

drum = {
    "steps":    [[False]*NUM_STEPS for _ in range(4)],   # [voice][step]
    "vol":      [0.8, 0.7, 0.6, 0.65],
    "running":  False,
    "bpm":      120.0,
    "cur_step": 0,
    "trig":     [False]*4,   # one-shot trigger flags set by sequencer thread
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
    active = synth["active_offset"]
    return int(synth["base_midi"]) + (active if active is not None else 0)

def current_play_freq_locked():
    return midi_to_freq(current_play_midi_locked())

def active_osc_locked():
    return synth["oscillators"][synth["active_osc"]]

noise_rng = np.random.default_rng()

# ═══════════════════════════════════════════════════════════════════════════════
#  DRUM SYNTHESIS  (simple 808/909 style)
# ═══════════════════════════════════════════════════════════════════════════════
# Per-voice state for synthesis (running phase/filter per voice)
drum_voice = [
    {"phase": 0.0, "fz": 0.0, "fz2": 0.0, "t": 0, "last_output": 0.0, "declick_from": 0.0, "declick_pos": 0},   # kick
    {"phase": 0.0, "fz": 0.0, "fz2": 0.0, "t": 0, "last_output": 0.0, "declick_from": 0.0, "declick_pos": 0},   # snare
    {"phase": 0.0, "fz": 0.0, "fz2": 0.0, "t": 0, "last_output": 0.0, "declick_from": 0.0, "declick_pos": 0},   # hihat
    {"phase": 0.0, "fz": 0.0, "fz2": 0.0, "t": 0, "last_output": 0.0, "declick_from": 0.0, "declick_pos": 0},   # top
]
drum_trig_sample = [-1]*4   # sample index when each voice was triggered (-1=off)

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

def apply_drum_attack(signal, t_samp):
    remaining = DRUM_ATTACK_SAMPLES - int(t_samp)
    if remaining <= 0 or len(signal) == 0:
        return signal
    fade = min(len(signal), remaining)
    ramp = (np.arange(fade, dtype=np.float32) + float(t_samp)) / float(DRUM_ATTACK_SAMPLES)
    signal[:fade] *= np.clip(ramp, 0.0, 1.0)
    return signal

def finalize_drum_chunk(signal, voice_idx, t_samp):
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
        samples_left = DRUM_LENGTHS[voice_idx] - (t_samp + np.arange(len(out), dtype=np.float32) + 1.0)
        tail_gain = np.clip(samples_left / float(DRUM_END_FADE_SAMPLES), 0.0, 1.0)
        out *= tail_gain
    if len(out) > 0:
        dv["last_output"] = float(out[-1])
    return out

def drum_synth_kick(t_samp, frames):
    """808 kick: sine with fast pitch drop + exponential decay."""
    if frames <= 0:
        return np.zeros(0, dtype=np.float32)
    dv  = drum_voice[0]
    age = t_samp + np.arange(frames, dtype=np.float32)
    freq = 40.0 + 120.0 * np.exp(-age / (0.08 * SAMPLE_RATE))
    amp = np.exp(-age / (0.45 * SAMPLE_RATE))
    phase = dv["phase"] + np.cumsum(freq / SAMPLE_RATE, dtype=np.float32)
    dv["phase"] = float(phase[-1] % 1.0)
    out = np.sin(2*np.pi*phase) * amp
    return finalize_drum_chunk(out, 0, t_samp)

def drum_synth_snare(t_samp, frames):
    """909 snare: pitched tone + noise burst, short decay."""
    if frames <= 0:
        return np.zeros(0, dtype=np.float32)
    dv  = drum_voice[1]
    age = t_samp + np.arange(frames, dtype=np.float32)
    amp = np.exp(-age / (0.12 * SAMPLE_RATE))
    phase = dv["phase"] + ((np.arange(frames, dtype=np.float32) + 1.0) * (220.0 / SAMPLE_RATE))
    dv["phase"] = float(phase[-1] % 1.0)
    tone = np.sin(2*np.pi*phase) * 0.5
    noise = noise_rng.uniform(-0.5, 0.5, frames).astype(np.float32)
    out = (tone + noise) * amp
    return finalize_drum_chunk(out, 1, t_samp)

def drum_synth_hihat(t_samp, frames):
    """Closed hihat: filtered noise, very short."""
    out = np.zeros(frames, dtype=np.float32)
    dv  = drum_voice[2]
    c   = np.exp(-2*np.pi * 8000.0 / SAMPLE_RATE)   # hi-pass-ish
    noise = noise_rng.uniform(-1.0, 1.0, frames).astype(np.float32)
    for i in range(frames):
        age   = t_samp + i
        amp   = np.exp(-age / (0.04 * SAMPLE_RATE))
        sample = noise[i]
        dv["fz"] = dv["fz"] * c + sample * (1-c)
        out[i] = (sample - dv["fz"]) * amp  # high-pass
    return finalize_drum_chunk(out, 2, t_samp)

def drum_synth_top(t_samp, frames):
    """Open top / cymbal: longer filtered noise."""
    out = np.zeros(frames, dtype=np.float32)
    dv  = drum_voice[3]
    c   = np.exp(-2*np.pi * 6000.0 / SAMPLE_RATE)
    noise = noise_rng.uniform(-1.0, 1.0, frames).astype(np.float32)
    for i in range(frames):
        age   = t_samp + i
        amp   = np.exp(-age / (0.25 * SAMPLE_RATE))
        sample = noise[i]
        dv["fz"] = dv["fz"] * c + sample * (1-c)
        out[i] = (sample - dv["fz"]) * amp
    return finalize_drum_chunk(out, 3, t_samp)

DRUM_SYNTHS  = [drum_synth_kick, drum_synth_snare, drum_synth_hihat, drum_synth_top]
DRUM_LENGTHS = [
    int(0.6  * SAMPLE_RATE),   # kick   ~600ms
    int(0.18 * SAMPLE_RATE),   # snare  ~180ms
    int(0.07 * SAMPLE_RATE),   # hihat   ~70ms
    int(0.35 * SAMPLE_RATE),   # top    ~350ms
]

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
        env       = synth["env"];   note   = synth["note_on"]
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
synth_dc_state = {"x1": 0.0, "y1": 0.0}
master_dc_state = {"x1": 0.0, "y1": 0.0}
master_limiter_state = {"gain": 1.0}

def apply_fx(signal):
    global delay_idx
    with synth_lock:
        drive_target    = synth["fx_drive"]
        mix_target      = synth["fx_delay_mix"]
        feedback_target = synth["fx_delay_feedback"]
        dtime_target    = synth["fx_delay_time"]
        drive_cur       = synth["fx_drive_current"]
        mix_cur         = synth["fx_delay_mix_current"]
        feedback_cur    = synth["fx_delay_feedback_current"]
        dtime_cur       = synth["fx_delay_time_current"]

    if max(drive_target, mix_target, drive_cur, mix_cur) <= 0.0001:
        return signal

    out = np.empty_like(signal)
    fx_alpha = 1.0 - np.exp(-1.0 / max(1.0, FX_SMOOTH_TIME * SAMPLE_RATE))
    delay_len = len(delay_buf)

    for i, dry in enumerate(signal):
        drive_cur += (drive_target - drive_cur) * fx_alpha
        mix_cur += (mix_target - mix_cur) * fx_alpha
        feedback_cur += (feedback_target - feedback_cur) * fx_alpha
        dtime_cur += (dtime_target - dtime_cur) * fx_alpha

        gain = 1.0 + drive_cur * 8.0
        norm = max(np.tanh(gain), 1e-6)
        delay_samples = max(1, int((0.08 + dtime_cur * 0.72) * SAMPLE_RATE))
        driven = np.tanh(dry * gain) / norm
        wet = delay_buf[(delay_idx - delay_samples) % delay_len]
        out[i] = driven * (1.0 - mix_cur) + wet * mix_cur
        fed = driven * 0.82 + wet * feedback_cur
        delay_buf[delay_idx] = soft_saturate_sample(fed, drive=1.02, ceiling=0.9)
        delay_idx = (delay_idx + 1) % delay_len

    with synth_lock:
        synth["fx_drive_current"] = drive_cur
        synth["fx_delay_mix_current"] = mix_cur
        synth["fx_delay_feedback_current"] = feedback_cur
        synth["fx_delay_time_current"] = dtime_cur

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
        drum["trig"] = [False] * 4
        drum_vols = drum["vol"][:]

    for v, pending in enumerate(pending_trigs):
        if pending:
            reset_drum_voice(v)
            drum_trig_sample[v] = _sample_clock

    for v in range(4):
        if drum_trig_sample[v] < 0:
            continue
        age = _sample_clock - drum_trig_sample[v]
        if age >= DRUM_LENGTHS[v]:
            drum_trig_sample[v] = -1
            continue
        remaining = min(frames, DRUM_LENGTHS[v] - age)
        chunk = DRUM_SYNTHS[v](age, remaining)
        drum_out[:remaining] += chunk * drum_vols[v] * 0.6

    drum_out *= 0.62
    synth_bus = (synth_live + loop_out) * MASTER_HEADROOM
    mixed = synth_bus + drum_out * 0.5
    mixed = dc_block(mixed, master_dc_state)
    mixed = apply_master_limiter(mixed, master_limiter_state)
    mixed *= OUTPUT_GAIN
    np.clip(mixed, -LIMIT_CEILING, LIMIT_CEILING, out=mixed)
    scope_show_drums = ui_state["scope_show_drums"]
    scope_sig = mixed if scope_show_drums else synth_bus

    outdata[:,0] = mixed
    if outdata.shape[1] > 1: outdata[:,1] = mixed

    with scope_lock:
        scope_buf.extend(scope_sig)
    _sample_clock += frames

# ═══════════════════════════════════════════════════════════════════════════════
#  SEQUENCER THREAD
# ═══════════════════════════════════════════════════════════════════════════════
def sequencer_thread():
    while True:
        with drum_lock:
            running = drum["running"]
            bpm     = drum["bpm"]
            step    = drum["cur_step"]
            steps   = [drum["steps"][v][step] for v in range(4)]

        if running:
            # fire triggers for active steps
            with drum_lock:
                for v in range(4):
                    if steps[v]:
                        drum["trig"][v] = True
                drum["cur_step"] = (step + 1) % NUM_STEPS

            beat_dur = 60.0 / bpm / 4.0   # 16th notes
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
        ]),
        ("LOOP + DRUMS", [
            (["R", "T", "P", "U"], "Record, overdub, toggle playback, or clear the loop."),
            (["TAB"], "Switch between synth focus and drum sequencer focus."),
            (["←", "→", "↑", "↓"], "Move around the drum grid while sequencer focus is active."),
            (["SPC", "r", "c", "C"], "Toggle step, run, clear row, or clear all."),
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

# ═══════════════════════════════════════════════════════════════════════════════
#  MAIN DRAW LOOP
# ═══════════════════════════════════════════════════════════════════════════════
def draw(stdscr):
    curses.curs_set(0); stdscr.nodelay(True); stdscr.keypad(True)
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

    stream = sd.OutputStream(samplerate=SAMPLE_RATE, blocksize=BLOCK_SIZE,
                              channels=2, dtype='float32', latency=0.2, callback=audio_cb)
    stream.start()

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
    settings_cursor = 0

    try:
        while True:
            ch = stdscr.getch()
            now = time.time()

            if ch == ord('H'):
                help_until = now + HELP_TIMEOUT

            # auto-release: curses has no keyup, so we release after timeout
            if held_note is not None and (now - last_note_t) > NOTE_TIMEOUT:
                with synth_lock:
                    synth["note_on"] = False
                    synth["active_offset"] = None
                    sync_pitch_locked()
                held_note = None

            if settings_open:
                if ch == ord('q'):
                    break
                elif ch in (27, ord('S')):
                    settings_open = False
                elif ch == curses.KEY_UP:
                    settings_cursor = (settings_cursor - 1) % 8
                elif ch == curses.KEY_DOWN:
                    settings_cursor = (settings_cursor + 1) % 8
                elif ch in (curses.KEY_LEFT, curses.KEY_RIGHT):
                    delta = -1 if ch == curses.KEY_LEFT else 1
                    if settings_cursor == 0:
                        with ui_lock:
                            ui_state["theme"] = (ui_state["theme"] + delta) % len(THEME_NAMES)
                    elif settings_cursor == 1:
                        with ui_lock:
                            ui_state["scope_show_drums"] = not ui_state["scope_show_drums"]
                    elif settings_cursor == 2:
                        with synth_lock:
                            synth["voices"] = int(clamp(synth["voices"] + delta, 1, MAX_VOICES))
                    elif settings_cursor == 3:
                        with synth_lock:
                            synth["active_osc"] = (synth["active_osc"] + delta) % 2
                    elif settings_cursor == 4:
                        with synth_lock:
                            active_osc_locked()["waveform"] = (active_osc_locked()["waveform"] + delta) % len(WAVEFORMS)
                    elif settings_cursor == 5:
                        with synth_lock:
                            active_osc_locked()["level"] = clamp(active_osc_locked()["level"] + delta * 0.05, 0.0, 1.0)
                    elif settings_cursor == 6:
                        with synth_lock:
                            active_osc_locked()["octave"] = int(clamp(active_osc_locked()["octave"] + delta, -2, 2))
                    elif settings_cursor == 7:
                        with synth_lock:
                            active_osc_locked()["detune_cents"] = clamp(active_osc_locked()["detune_cents"] + delta * 2.0, -24.0, 24.0)
            else:
                if ch == ord('\t'):
                    focus = "seq" if focus=="synth" else "synth"
                    with synth_lock:
                        synth["note_on"] = False
                        synth["active_offset"] = None
                        sync_pitch_locked()
                    held_note = None
                elif ch == ord('S'):
                    settings_open = True
                elif focus == "synth":
                    if ch in KEYBOARD_OFFSETS:
                        with synth_lock:
                            synth["active_offset"] = KEYBOARD_OFFSETS[ch]
                            synth["note_on"] = True
                            sync_pitch_locked()
                            play_midi = current_play_midi_locked()
                        last_note   = midi_to_name(play_midi)
                        held_note   = ch
                        last_note_t = now
                    elif ch == ord(' '):
                        with synth_lock:
                            synth["note_on"] = False
                            synth["active_offset"] = None
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
                                loop["buffer"].fill(0.0)
                                loop["length"] = 0
                                loop["write_pos"] = 0
                                loop["read_pos"] = 0
                                loop["recording"] = True
                                loop["playing"] = False
                                loop["overdub"] = False
                                loop["has_audio"] = False
                    elif ch == ord('T'):
                        with loop_lock:
                            if loop["recording"] and loop["overdub"]:
                                loop["recording"] = False
                                loop["overdub"] = False
                            elif loop["has_audio"]:
                                loop["recording"] = True
                                loop["playing"] = True
                                loop["overdub"] = True
                                loop["write_pos"] = loop["read_pos"]
                    elif ch == ord('P'):
                        with loop_lock:
                            if loop["has_audio"]:
                                loop["playing"] = not loop["playing"]
                    elif ch == ord('U'):
                        with loop_lock:
                            loop["buffer"].fill(0.0)
                            loop["length"] = 0
                            loop["write_pos"] = 0
                            loop["read_pos"] = 0
                            loop["recording"] = False
                            loop["playing"] = False
                            loop["overdub"] = False
                            loop["has_audio"] = False
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
                        seq_cursor_v = (seq_cursor_v - 1) % 4
                    elif ch == curses.KEY_DOWN:
                        seq_cursor_v = (seq_cursor_v + 1) % 4
                    elif ch == curses.KEY_LEFT:
                        seq_cursor_s = (seq_cursor_s - 1) % NUM_STEPS
                    elif ch == curses.KEY_RIGHT:
                        seq_cursor_s = (seq_cursor_s + 1) % NUM_STEPS
                    elif ch == ord(' '):
                        with drum_lock:
                            drum["steps"][seq_cursor_v][seq_cursor_s] = \
                                not drum["steps"][seq_cursor_v][seq_cursor_s]
                    elif ch == ord('\n') or ch == ord('r'):
                        with drum_lock: drum["running"] = not drum["running"]
                    elif ch == ord('c'):
                        with drum_lock: drum["steps"][seq_cursor_v] = [False]*NUM_STEPS
                    elif ch == ord('C'):
                        with drum_lock: drum["steps"] = [[False]*NUM_STEPS for _ in range(4)]
                    elif ch == ord(','):
                        with drum_lock: drum["bpm"] = clamp(drum["bpm"]-1, 40, 300)
                    elif ch == ord('.'):
                        with drum_lock: drum["bpm"] = clamp(drum["bpm"]+1, 40, 300)
                    elif ch == ord('<'):
                        with drum_lock: drum["bpm"] = clamp(drum["bpm"]-5, 40, 300)
                    elif ch == ord('>'):
                        with drum_lock: drum["bpm"] = clamp(drum["bpm"]+5, 40, 300)
                    elif ch == ord('-'):
                        with drum_lock: drum["vol"][seq_cursor_v]=clamp(drum["vol"][seq_cursor_v]-0.05,0,1)
                    elif ch == ord('='):
                        with drum_lock: drum["vol"][seq_cursor_v]=clamp(drum["vol"][seq_cursor_v]+0.05,0,1)
                    elif ch == ord('1'):
                        with drum_lock:
                            drum["steps"][0] = [True,False,False,False, True,False,False,False,
                                                True,False,False,False, True,False,False,False]
                            drum["steps"][1] = [False,False,False,False, True,False,False,False,
                                                False,False,False,False, True,False,False,False]
                            drum["steps"][2] = [True,False,True,False]*4
                            drum["steps"][3] = [False]*16
                    elif ch == ord('2'):
                        with drum_lock:
                            drum["steps"][0] = [True,False,False,False, False,False,False,False,
                                                True,False,False,True,  False,False,False,False]
                            drum["steps"][1] = [False,False,False,False, True,False,False,False,
                                                False,False,False,False, True,False,False,True]
                            drum["steps"][2] = [True]*16
                            drum["steps"][3] = [False,False,False,False, False,False,False,False,
                                                True,False,False,False,  False,False,True,False]

            # ── snapshot ───────────────────────────
            with synth_lock:
                vol=synth["volume"]; freq=current_play_freq_locked()
                base_midi=synth["base_midi"]; voices=synth["voices"]
                atk=synth["attack"]; rel=synth["release"]; env=synth["env"]
                note=synth["note_on"]; gate=synth["gate_mode"]
                lfo_wf=synth["lfo_wave"]; lfo_rate=synth["lfo_rate"]
                lfo_dep=synth["lfo_depth"]; lfo_tgt=synth["lfo_target"]
                filt_on=synth["filter_on"]; cutoff=synth["cutoff"]
                res=synth["resonance"]; lfo_ph=synth["lfo_phase"]
                fx_drive=synth["fx_drive"]; fx_mix=synth["fx_delay_mix"]
                fx_feedback=synth["fx_delay_feedback"]; fx_time=synth["fx_delay_time"]
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

            with ui_lock:
                theme=ui_state["theme"]
                scope_show_drums=ui_state["scope_show_drums"]

            with drum_lock:
                d_steps   = [row[:] for row in drum["steps"]]
                d_running = drum["running"]
                d_bpm     = drum["bpm"]
                d_cur     = drum["cur_step"]
                d_vol     = drum["vol"][:]

            with scope_lock:
                snap = list(scope_buf)

            h, w = stdscr.getmaxyx()
            stdscr.erase()

            C  = [curses.color_pair(i) for i in range(15)]
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
            DRUM_H   = 7
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
            safe_addstr(stdscr,10,24,f"XR {xruns:02d}", C[6])

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

            # ── KEYS ───────────────────────────────
            safe_addstr(stdscr,29,2,"KEYS",C[2])
            safe_addstr(stdscr,29,8,"a w s e d f t g y h u j k",C[3])
            key_status = f" ♪ {last_note} " if note else (" ♪ FREE " if gate==1 else f" ROOT {base_note_name} ")
            safe_addstr(stdscr,30,2,key_status,(C[4]|B) if (note or gate==1) else C[3])
            safe_addstr(stdscr,30,16,"R rec  T dub  P play  U clear",C[6]|DIM)

            # ══════════════════════════════════════
            #  SCOPE  (top-right)
            # ══════════════════════════════════════
            sw        = max(4, w - scope_x - 1)
            scope_top = 1
            scope_bot = drum_top - 1
            sh        = max(1, scope_bot - scope_top)

            safe_addstr(stdscr, scope_top, scope_x, "SCOPE " + "─"*(sw-6), C[2])
            lines = render_scope(snap, sw, sh-1)
            for row, line in enumerate(lines):
                safe_addstr(stdscr, scope_top+1+row, scope_x, line, scope_attr)

            scope_lbl = "MIX" if scope_show_drums else "SYNTH"
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

            # step numbers header
            step_x0 = 6
            for s in range(NUM_STEPS):
                sx = step_x0 + s*3
                # beat marker every 4 steps
                if s % 4 == 0:
                    safe_addstr(stdscr, drum_top+1, sx, f"{s+1:2d} ", C[2])
                else:
                    safe_addstr(stdscr, drum_top+1, sx, "·  ", C[2]|DIM)

            # playhead indicator
            ph_x = step_x0 + d_cur*3
            if d_running:
                safe_addstr(stdscr, drum_top+1, ph_x, "▼", C[4]|B)

            # voice rows
            VOICE_COLORS = [10, 11, 12, 13]
            for v in range(4):
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
                    sx = step_x0 + s*3
                    on = d_steps[v][s]
                    is_cur_step = d_running and s==d_cur
                    is_cursor   = (focus=="seq" and seq_cursor_v==v and seq_cursor_s==s)

                    if is_cursor:
                        if on:
                            ch_str = "[■]"
                            attr   = C[VOICE_COLORS[v]] | B
                        else:
                            ch_str = "[ ]"
                            attr   = C[8] | B
                    elif on:
                        ch_str = " ■ " if not is_cur_step else "►■◄"
                        attr   = (C[VOICE_COLORS[v]] | B) if is_cur_step else C[VOICE_COLORS[v]]
                    else:
                        ch_str = " · "
                        attr   = (C[5]|DIM) if is_cur_step else (C[3]|DIM)

                    safe_addstr(stdscr, row, sx, ch_str, attr)

                # vol bar at end
                vol_x = step_x0 + NUM_STEPS*3 + 1
                safe_addstr(stdscr, row, vol_x, hbar(vv, 6), C[5] if is_selected else (C[5]|DIM))
                safe_addstr(stdscr, row, vol_x+7, "-/=" if is_selected else "   ", C[6])

            # footer
            if settings_open:
                ftr = " ↑↓ select | ←→ change | S/Esc close | H hold help | q quit "
            elif focus == "synth":
                ftr = " TAB=drums | 1/2 osc | R/T/P/U loop | S settings | q quit "
            else:
                ftr = " TAB=synth | ←→↑↓ move | SPC toggle | r run | S settings | H hold help | q quit "
            safe_addstr(stdscr, h-1, 0, ftr[:w-1], C[1])

            if show_help:
                draw_help_overlay(stdscr, h, w, scope_attr, C, B, DIM)

            if settings_open:
                box_w = min(48, max(30, w - 10))
                box_h = 13
                box_x = max(2, (w - box_w) // 2)
                box_y = max(2, (h - box_h) // 2)
                draw_box(stdscr, box_y, box_x, box_w, box_h, "SETTINGS", scope_attr|B)
                rows = [
                    f"Theme        {THEME_NAMES[theme]}",
                    f"Scope source {'MIX' if scope_show_drums else 'SYNTH ONLY'}",
                    f"Voices       {int(voices)}",
                    f"Edit osc     OSC{active_osc + 1}",
                    f"Waveform     {WAVEFORMS[oscillators[active_osc]['waveform']]}",
                    f"Osc level    {int(oscillators[active_osc]['level'] * 100):3d}%",
                    f"Osc octave   {oscillators[active_osc]['octave']:+d}",
                    f"Osc detune   {oscillators[active_osc]['detune_cents']:+5.1f}c",
                ]
                for idx, row_text in enumerate(rows):
                    attr = (C[8]|B) if idx == settings_cursor else C[3]
                    safe_addstr(stdscr, box_y + 2 + idx, box_x + 2, row_text.ljust(box_w-4), attr)
                safe_addstr(stdscr, box_y + box_h - 2, box_x + 2, "Use ←→ to change, ↑↓ to select, S/Esc to close", C[6])

            stdscr.refresh()
            time.sleep(0.04)

    finally:
        stream.stop(); stream.close()

curses.wrapper(draw)
PYEOF

# ── bootstrap venv on first run ──────────
if [ ! -f "$VENV_DIR/bin/python" ]; then
  echo "First run: setting up virtual environment..."
  python3 -m venv "$VENV_DIR"
  "$VENV_DIR/bin/pip" install --quiet numpy sounddevice
  echo "Done! Starting synth..."
  sleep 1
fi

exec "$VENV_DIR/bin/python" "$PY_SCRIPT"
