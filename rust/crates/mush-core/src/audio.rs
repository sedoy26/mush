//! Real-time audio engine with lock-free synchronization.
//!
//! Key design principles:
//! - Audio callback NEVER acquires a blocking lock
//! - Sample rate is dynamic (from CPAL device negotiation)
//! - UI→Audio: ArcSwap for params, SPSC queue for commands (consumer moved into closure)
//! - Audio→UI: Atomics for reactive state, pre-allocated scope buffers
//! - All DSP is sample-rate-aware (no magic constants)
//! - Soft limiting is continuous (no conditional clipping)
//! - Anti-aliased oscillators via PolyBLEP
//!
//! NOTE: The AudioEngine uses try_lock() for export/import operations.
//! This is non-blocking (audio callback never waits) but not strictly lock-free.
//! Brief audio glitches (silence) may occur during project save/load.

use std::{
    collections::VecDeque,
    f32::consts::PI,
    path::Path,
    sync::Arc,
};

use anyhow::{Context, Result};
use crossbeam_queue::SegQueue;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;

use crate::{
    audio_bridge::{AudioBridge, AudioCommand, SampleParams},
    dsp::{self, SmoothEnvelope, SimpleReverb, DENORMAL_PREVENTION, MAX_FEEDBACK},
    project_io,
    state::{
        audio::AudioDeviceSelection,
        drums::{get_bank, DrumState, DrumVoice},
        synth::{midi_to_freq, GateMode, OscillatorState, SynthState, Waveform},
        MAX_VOICES, NUM_DRUM_VOICES, NUM_STEPS,
    },
};

// Constants that don't depend on sample rate
const MAX_LOOP_SECONDS: usize = 8;
const LIMIT_CEILING: f32 = 0.85;   // More aggressive ceiling
const LIMIT_DRIVE: f32 = 2.0;      // Stronger saturation curve
const OUTPUT_GAIN: f32 = 0.35;     // Reduced master output
const SYNTH_BUS_GAIN: f32 = 0.55;  // Reduced synth
const SAMPLE_BUS_GAIN: f32 = 0.55; // Sample bus independent from synth volume knob
const DRUM_BUS_GAIN: f32 = 0.28;   // Drums post-synth-FX; bus still soft-limited
const DRUM_VOICE_GAIN: f32 = 0.40; // Per-voice trim; kick multiplies by KICK_VOICE_GAIN_MUL
const KICK_VOICE_GAIN_MUL: f32 = 1.52; // Extra fader path for kick vs other voices
const KICK_ATTACK_MS: f32 = 2.8;   // Short enough for punch; tail fade handles cutoff
const KICK_LEVEL: f32 = 0.62;      // Base synthesis gain; kick_makeup() evens banks
// Filter frequency range (exponential mapping)
const FILTER_MIN_FREQ: f32 = 20.0;    // 20 Hz minimum
const FILTER_MAX_FREQ: f32 = 18000.0; // 18 kHz maximum (leave headroom below Nyquist)

pub struct AudioRuntime {
    pub stream: cpal::Stream,
}

/// Shared audio subsystem with lock-free bridge to UI.
pub struct SharedAudio {
    /// Lock-free bridge for UI↔Audio communication
    pub bridge: Arc<AudioBridge>,
    /// Audio engine (mutex used for export/import, try_lock in callback)
    engine: Arc<Mutex<AudioEngine>>,
    /// Base directory for recordings
    base_dir: std::path::PathBuf,
}

impl SharedAudio {
    pub fn new(bridge: Arc<AudioBridge>, base_dir: &Path) -> Self {
        Self {
            bridge,
            // Default sample rate of 44100, will be updated when stream starts
            engine: Arc::new(Mutex::new(AudioEngine::new(44100.0))),
            base_dir: base_dir.to_path_buf(),
        }
    }

    pub fn start(
        &self,
        selection: &AudioDeviceSelection,
        command_rx: rtrb::Consumer<AudioCommand>,
        midi_sample_q: Option<Arc<SegQueue<AudioCommand>>>,
    ) -> Result<AudioRuntime> {
        let host = cpal::default_host();
        let device = select_output_device(&host, selection)?;
        let config = device.default_output_config().context("output config")?;
        let sample_format = config.sample_format();
        
        // Get actual sample rate from device config
        // SampleRate implements Into<u32>
        let sample_rate: f32 = {
            let rate: u32 = config.sample_rate().into();
            rate as f32
        };
        
        let stream_config = config.config();
        
        // Update engine with actual sample rate
        {
            let mut engine = self.engine.lock();
            engine.set_sample_rate(sample_rate);
        }

        let bridge_err = Arc::clone(&self.bridge);
        let err_fn = move |_err| {
            // Can't do much here since we can't lock - just increment xruns atomically
            bridge_err.reactive.xruns.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        };

        // Command consumer is wrapped in Mutex for Send + interior mutability.
        // IMPORTANT: This mutex is ONLY accessed from the audio callback thread.
        // There is never contention. The lock/unlock is just atomic pointer swap.
        // This is not architecturally pure "lock-free" but practically equivalent.
        let command_rx = Arc::new(Mutex::new(command_rx));
        
        let bridge = Arc::clone(&self.bridge);
        let engine = Arc::clone(&self.engine);
        let command_rx_f32 = Arc::clone(&command_rx);
        let midi_f32 = midi_sample_q.clone();

        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &stream_config,
                move |data: &mut [f32], _| {
                    let mut rx = command_rx_f32.lock();
                    render_callback(data, &bridge, &engine, &mut rx, &midi_f32);
                },
                err_fn,
                None,
            )?,
            cpal::SampleFormat::I16 => {
                let bridge = Arc::clone(&self.bridge);
                let engine = Arc::clone(&self.engine);
                let command_rx_i16 = Arc::clone(&command_rx);
                let midi_i16 = midi_sample_q.clone();
                device.build_output_stream(
                    &stream_config,
                    move |data: &mut [i16], _| {
                        let mut rx = command_rx_i16.lock();
                        render_callback_i16(data, &bridge, &engine, &mut rx, &midi_i16);
                    },
                    err_fn,
                    None,
                )?
            }
            cpal::SampleFormat::U16 => {
                let bridge = Arc::clone(&self.bridge);
                let engine = Arc::clone(&self.engine);
                let command_rx_u16 = Arc::clone(&command_rx);
                let midi_u16 = midi_sample_q.clone();
                device.build_output_stream(
                    &stream_config,
                    move |data: &mut [u16], _| {
                        let mut rx = command_rx_u16.lock();
                        render_callback_u16(data, &bridge, &engine, &mut rx, &midi_u16);
                    },
                    err_fn,
                    None,
                )?
            }
            other => anyhow::bail!("unsupported sample format: {other:?}"),
        };

        stream.play()?;
        Ok(AudioRuntime { stream })
    }

    pub fn flush_recording_if_needed(&self) -> Result<Option<String>> {
        let mut engine = self.engine.lock();
        if engine.pending_record_flush {
            let path = project_io::next_wav_path(&self.base_dir)?;
            engine.write_wav(&path)?;
            engine.pending_record_flush = false;
            Ok(Some(path.display().to_string()))
        } else {
            Ok(None)
        }
    }

    /// Export the current loop buffer for saving with project.
    /// Returns Some((samples, length, sample_rate)) if there's recorded audio.
    pub fn export_loop(&self) -> Option<(Vec<f32>, usize, u32)> {
        let engine = self.engine.lock();
        engine.export_loop_data()
    }

    /// Import loop buffer samples when loading a project.
    pub fn import_loop(&self, samples: &[f32], length: usize) {
        let mut engine = self.engine.lock();
        engine.import_loop_data(samples, length);
    }

    pub fn export_sample_loop(&self) -> Option<(Vec<f32>, usize, u32)> {
        let engine = self.engine.lock();
        engine.export_sample_loop_data()
    }

    pub fn import_sample_loop(&self, samples: &[f32], length: usize) {
        let mut engine = self.engine.lock();
        engine.import_sample_loop_data(samples, length);
    }
}

fn select_output_device(
    host: &cpal::Host,
    selection: &AudioDeviceSelection,
) -> Result<cpal::Device> {
    match selection {
        AudioDeviceSelection::DefaultSystem => host
            .default_output_device()
            .context("no default output device"),
        AudioDeviceSelection::Named(name) => {
            for device in host.output_devices().context("enumerate output devices")? {
                if device_display_name(&device) == *name {
                    return Ok(device);
                }
            }
            host.default_output_device()
                .context("named output missing and no default output")
        }
    }
}

fn device_display_name(device: &cpal::Device) -> String {
    device
        .description()
        .map(|description| description.name().to_string())
        .unwrap_or_else(|_| "default".to_string())
}

/// Main render callback - MUST NOT BLOCK.
/// Command consumer is owned by this closure (moved in at stream creation).
fn drain_audio_commands(
    engine: &mut AudioEngine,
    command_rx: &mut rtrb::Consumer<AudioCommand>,
    midi_sample_q: &Option<Arc<SegQueue<AudioCommand>>>,
) {
    while let Ok(cmd) = command_rx.pop() {
        engine.process_command(cmd);
    }
    if let Some(mq) = midi_sample_q {
        while let Some(cmd) = mq.pop() {
            engine.process_command(cmd);
        }
    }
}

fn render_callback(
    data: &mut [f32],
    bridge: &Arc<AudioBridge>,
    engine: &Arc<Mutex<AudioEngine>>,
    command_rx: &mut rtrb::Consumer<AudioCommand>,
    midi_sample_q: &Option<Arc<SegQueue<AudioCommand>>>,
) {
    let channels = 2usize;
    let frames = data.len() / channels;
    let mut mono = vec![0.0f32; frames];

    // Try to get the engine lock - if contended, output silence (rare edge case)
    // This only contends during project save/load (export_loop/import_loop)
    if let Some(mut engine) = engine.try_lock() {
        // 1) Load UI snapshot. 2) Copy sample bus into the engine (so `render_sample` reads stable data).
        // 3) Drain commands — `SampleNoteOn` may replace `sample_params` with a fresh snapshot before arming.
        let params = bridge.load_params();
        engine.sample_params = params.sample.clone();
        drain_audio_commands(&mut engine, command_rx, midi_sample_q);

        // Render audio
        let (synth_samples, drum_samples) = engine.render_into(&params, &mut mono);
        
        // Update reactive state via atomics (lock-free)
        bridge.reactive.master.store(peak_level(&mono));
        bridge.reactive.synth.store(peak_level(&synth_samples));
        bridge.reactive.drums.store(peak_level(&drum_samples));
        bridge.reactive.env.store(engine.env);
        bridge.reactive.freq_current.store(engine.freq_current);
        bridge.reactive.lfo_phase.store(engine.lfo_phase);
        bridge.reactive.last_output.store(engine.last_output);
        
        // Update kick/snare/hat reactive based on actual drum engine triggers (not UI state)
        if engine.drum_triggered[0] {
            bridge.reactive.kick.store(1.0);
        } else {
            let current = bridge.reactive.kick.load();
            bridge.reactive.kick.store((current * 0.92).max(0.0));
        }
        if engine.drum_triggered[1] || engine.drum_triggered[2] {
            bridge.reactive.snare.store(1.0);
        } else {
            let current = bridge.reactive.snare.load();
            bridge.reactive.snare.store((current * 0.92).max(0.0));
        }
        if engine.drum_triggered[3] || engine.drum_triggered[4] || engine.drum_triggered[5] {
            bridge.reactive.hat.store(1.0);
        } else {
            let current = bridge.reactive.hat.load();
            bridge.reactive.hat.store((current * 0.92).max(0.0));
        }
        
        let sample_active = engine.sample_voices.iter().any(|v| v.active && v.env > 0.01);
        bridge.reactive.note.store(if engine.env > 0.01 || sample_active {
            1.0
        } else {
            0.0
        });
        
        // Update scope buffer (uses pre-allocated pool, no allocation)
        bridge.update_scope(&mono, &synth_samples);
        
        // Update looper state atomics
        bridge.reactive.loop_length.store(engine.loop_len as u64, std::sync::atomic::Ordering::Relaxed);
        bridge.reactive.loop_read_pos.store(engine.loop_read as u64, std::sync::atomic::Ordering::Relaxed);
        bridge.reactive.loop_write_pos.store(engine.loop_write as u64, std::sync::atomic::Ordering::Relaxed);
        bridge.reactive.loop_has_audio.store(
            if engine.loop_len > (engine.sample_rate as usize / 12) { 1 } else { 0 },
            std::sync::atomic::Ordering::Relaxed,
        );
        bridge
            .reactive
            .sample_loop_length
            .store(engine.sloop_len as u64, std::sync::atomic::Ordering::Relaxed);
        bridge
            .reactive
            .sample_loop_read_pos
            .store(engine.sloop_read as u64, std::sync::atomic::Ordering::Relaxed);
        bridge
            .reactive
            .sample_loop_write_pos
            .store(engine.sloop_write as u64, std::sync::atomic::Ordering::Relaxed);
        bridge.reactive.sample_loop_has_audio.store(
            if engine.sloop_len > (engine.sample_rate as usize / 12) {
                1
            } else {
                0
            },
            std::sync::atomic::Ordering::Relaxed,
        );

        // Update drum sequencer state atomics
        bridge.reactive.drum_step.store(engine.seq_step as u32, std::sync::atomic::Ordering::Relaxed);
        bridge.reactive.chain_position.store(engine.chain_position as u32, std::sync::atomic::Ordering::Relaxed);
        // Calculate playing pattern for UI display
        let playing_pattern = if params.drums.chain_mode && !params.drums.chain.is_empty() {
            params.drums.chain[engine.chain_position % params.drums.chain.len()]
        } else {
            params.drums.current_pattern
        };
        bridge.reactive.playing_pattern.store(playing_pattern as u32, std::sync::atomic::Ordering::Relaxed);
        for i in 0..6 {
            bridge.reactive.drum_triggers[i].store(
                if engine.drum_triggered[i] { 1 } else { 0 },
                std::sync::atomic::Ordering::Relaxed,
            );
        }
    }

    // Write mono to stereo output with final safety clamp
    for (frame, sample) in mono.into_iter().enumerate() {
        let base = frame * channels;
        let clamped = sample.clamp(-0.98, 0.98);
        data[base] = clamped;
        if channels > 1 {
            data[base + 1] = clamped;
        }
    }
}

fn render_callback_i16(
    data: &mut [i16],
    bridge: &Arc<AudioBridge>,
    engine: &Arc<Mutex<AudioEngine>>,
    command_rx: &mut rtrb::Consumer<AudioCommand>,
    midi_sample_q: &Option<Arc<SegQueue<AudioCommand>>>,
) {
    let mut scratch = vec![0.0f32; data.len() / 2];
    render_callback_core(&mut scratch, bridge, engine, command_rx, midi_sample_q);
    for (frame, sample) in scratch.into_iter().enumerate() {
        let value = (sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16;
        let base = frame * 2;
        data[base] = value;
        data[base + 1] = value;
    }
}

fn render_callback_u16(
    data: &mut [u16],
    bridge: &Arc<AudioBridge>,
    engine: &Arc<Mutex<AudioEngine>>,
    command_rx: &mut rtrb::Consumer<AudioCommand>,
    midi_sample_q: &Option<Arc<SegQueue<AudioCommand>>>,
) {
    let mut scratch = vec![0.0f32; data.len() / 2];
    render_callback_core(&mut scratch, bridge, engine, command_rx, midi_sample_q);
    for (frame, sample) in scratch.into_iter().enumerate() {
        let value = (((sample.clamp(-1.0, 1.0) * 0.5) + 0.5) * u16::MAX as f32) as u16;
        let base = frame * 2;
        data[base] = value;
        data[base + 1] = value;
    }
}

/// Core render function for i16/u16 callbacks
fn render_callback_core(
    mono: &mut [f32],
    bridge: &Arc<AudioBridge>,
    engine: &Arc<Mutex<AudioEngine>>,
    command_rx: &mut rtrb::Consumer<AudioCommand>,
    midi_sample_q: &Option<Arc<SegQueue<AudioCommand>>>,
) {
    if let Some(mut engine) = engine.try_lock() {
        let params = bridge.load_params();
        engine.sample_params = params.sample.clone();
        drain_audio_commands(&mut engine, command_rx, midi_sample_q);
        engine.render_into(&params, mono);
    }
}

#[derive(Clone, Copy)]
struct DrumVoiceState {
    age: Option<usize>,
    last_value: f32,        // Track last output for crossfade on retrigger
    crossfade_from: f32,    // Value to crossfade from on retrigger
    crossfade_samples: usize, // Samples remaining in crossfade
    /// Integrated phase for kick (frequency sweeps must use ∫2πf dt, not f(t)·t)
    kick_phase: f32,
}

const DRUM_CROSSFADE_SAMPLES: usize = 64; // ~1.3ms at 48kHz - fast enough to not lose transient
/// Fade out the last few ms of each drum voice — hard age cutoff vs ongoing envelope causes a click.
const DRUM_TAIL_FADE_SAMPLES: usize = 220; // ~4.6ms @ 48kHz

/// Banks with low fundamental / weak click read much quieter at the same peak; lift in a bounded way.
#[inline]
fn kick_makeup(p: &crate::state::drums::KickParams) -> f32 {
    // Sub-heavy kicks (low base_freq) need more level to match perceived punch on typical speakers.
    let ref_hz = 52.0f32;
    let freq_lift = (ref_hz / p.base_freq.max(26.0)).sqrt().clamp(1.0, 2.15);
    // Small `click` → little energy in the audible attack band.
    let click_lift = (1.05 + (0.11 - p.click).max(0.0) * 2.8).clamp(1.0, 1.48);
    (freq_lift * click_lift).clamp(1.0, 2.25)
}

/// Kick body: phase must integrate 2πf(t)/sr each sample when f sweeps (not 2πf(t)·t).
fn kick_sample_osc(
    sample_rate: f32,
    phase: &mut f32,
    t: f32,
    noise: f32,
    p: &crate::state::drums::KickParams,
) -> f32 {
    let pitch_env = (-t / p.pitch_decay).exp();
    let freq = p.base_freq + (p.sweep_freq - p.base_freq) * pitch_env;
    *phase += 2.0 * PI * freq / sample_rate;
    let tone = phase.sin();
    let click_env = (-t / (p.click_decay * 0.5)).exp();
    let click = noise * p.click * 0.48 * click_env;
    let amp_env = (-t / p.amp_decay).exp();
    // Smooth attack ramp using cosine curve (softer than linear)
    let attack_time = KICK_ATTACK_MS / 1000.0;
    let attack_env = if t < attack_time {
        0.5 - 0.5 * (PI * t / attack_time).cos() // Cosine fade-in
    } else {
        1.0
    };
    let makeup = kick_makeup(p);
    ((tone + click) * amp_env * attack_env * KICK_LEVEL * makeup).clamp(-1.0, 1.0)
}

const SAMPLE_POLY: usize = 8;

#[derive(Clone, Copy, Default)]
struct SampleVoice {
    active: bool,
    note: u8,
    vel: f32,
    phase: f64,
    env: f32,
    releasing: bool,
}

struct AudioEngine {
    sample_rate: f32,
    sample_clock: u64,
    sample_voices: [SampleVoice; SAMPLE_POLY],
    /// Sample buffer + trim/gain used for playback (synced from bridge each block; `SampleNoteOn` may refresh).
    sample_params: SampleParams,
    // Synth envelope - now properly handles retrigger from current value
    envelope: SmoothEnvelope,
    env: f32,  // Keep for UI feedback
    freq_current: f32,
    lfo_phase: f32,
    filter_z1: f32,
    _filter_z2: f32,  // Reserved for resonant filter (unused currently)
    current_cutoff: f32,      // Smoothed filter cutoff (prevents zipper noise)
    current_gain: f32,        // Smoothed gain (prevents zipper noise)
    warmth_z: f32,   // Dedicated state for warmth FX filter
    air_z: f32,      // Dedicated state for air FX differentiation
    delay_buf: Vec<f32>,
    delay_idx: usize,
    // Improved reverb with allpass diffusers
    reverb: SimpleReverb,
    loop_buf: Vec<f32>,
    loop_len: usize,
    loop_write: usize,
    loop_read: usize,
    /// Sample-tab performance loop buffer (records `render_sample` output only).
    sloop_buf: Vec<f32>,
    sloop_len: usize,
    sloop_write: usize,
    sloop_read: usize,
    last_output: f32,
    osc_phases: [[f32; 6]; 2],
    drum_state: [DrumVoiceState; NUM_DRUM_VOICES],
    drum_noise: u32,
    seq_accum: f32,
    seq_step: usize,
    /// Current position in pattern chain
    chain_position: usize,
    /// Cached chain length (to detect changes)
    chain_len: usize,
    rec_buffer: Vec<f32>,
    was_recording: bool,
    pending_record_flush: bool,
    scope: VecDeque<f32>,
    // Looper state managed by commands
    loop_recording: bool,
    loop_overdub: bool,
    loop_playing: bool,
    loop_gain: f32,
    loop_speed: f32,
    // Track which drums triggered this frame for UI feedback
    drum_triggered: [bool; 6],
    // Track if we already processed a clear request
    last_clear_seen: bool,
    sloop_recording: bool,
    sloop_overdub: bool,
    sloop_playing: bool,
    sloop_gain: f32,
    sloop_last_clear_seen: bool,
}

impl AudioEngine {
    fn new(sample_rate: f32) -> Self {
        Self {
            sample_rate,
            sample_clock: 0,
            envelope: SmoothEnvelope::new(0.01, 0.3, sample_rate),
            env: 0.0,
            freq_current: midi_to_freq(60),
            lfo_phase: 0.0,
            filter_z1: 0.0,
            _filter_z2: 0.0,
            current_cutoff: 0.5,      // Start at mid-range cutoff
            current_gain: 0.7,        // Start at typical volume
            warmth_z: 0.0,
            air_z: 0.0,
            delay_buf: vec![0.0; (sample_rate * 2.0) as usize], // 2 second max delay
            delay_idx: 0,
            reverb: SimpleReverb::new(sample_rate, 0.5, 0.3),
            loop_buf: vec![0.0; (sample_rate as usize) * MAX_LOOP_SECONDS],
            loop_len: 0,
            loop_write: 0,
            loop_read: 0,
            sloop_buf: vec![0.0; (sample_rate as usize) * MAX_LOOP_SECONDS],
            sloop_len: 0,
            sloop_write: 0,
            sloop_read: 0,
            last_output: 0.0,
            osc_phases: [[0.0; 6]; 2],
            drum_state: [
                DrumVoiceState { age: None, last_value: 0.0, crossfade_from: 0.0, crossfade_samples: 0, kick_phase: 0.0 },
                DrumVoiceState { age: None, last_value: 0.0, crossfade_from: 0.0, crossfade_samples: 0, kick_phase: 0.0 },
                DrumVoiceState { age: None, last_value: 0.0, crossfade_from: 0.0, crossfade_samples: 0, kick_phase: 0.0 },
                DrumVoiceState { age: None, last_value: 0.0, crossfade_from: 0.0, crossfade_samples: 0, kick_phase: 0.0 },
                DrumVoiceState { age: None, last_value: 0.0, crossfade_from: 0.0, crossfade_samples: 0, kick_phase: 0.0 },
                DrumVoiceState { age: None, last_value: 0.0, crossfade_from: 0.0, crossfade_samples: 0, kick_phase: 0.0 },
            ],
            drum_noise: 0x1234_5678,
            seq_accum: 0.0,
            seq_step: 0,
            chain_position: 0,
            chain_len: 0,
            rec_buffer: Vec::new(),
            was_recording: false,
            pending_record_flush: false,
            scope: VecDeque::with_capacity(2048),
            loop_recording: false,
            loop_overdub: false,
            loop_playing: false,
            loop_gain: 0.7,
            loop_speed: 1.0,
            drum_triggered: [false; 6],
            last_clear_seen: false,
            sloop_recording: false,
            sloop_overdub: false,
            sloop_playing: false,
            sloop_gain: 0.7,
            sloop_last_clear_seen: false,
            sample_voices: [SampleVoice::default(); SAMPLE_POLY],
            sample_params: SampleParams::default(),
        }
    }

    fn set_sample_rate(&mut self, sample_rate: f32) {
        if (self.sample_rate - sample_rate).abs() < 1.0 {
            return; // No change needed
        }
        self.sample_rate = sample_rate;
        // Resize buffers for new sample rate
        self.delay_buf.resize((sample_rate * 2.0) as usize, 0.0);
        self.delay_idx = 0;
        // Recreate reverb with new sample rate
        self.reverb = SimpleReverb::new(sample_rate, 0.5, 0.3);
        self.loop_buf.resize((sample_rate as usize) * MAX_LOOP_SECONDS, 0.0);
        self.sloop_buf.resize((sample_rate as usize) * MAX_LOOP_SECONDS, 0.0);
        // Update envelope coefficients
        self.envelope = SmoothEnvelope::new(0.01, 0.3, sample_rate);
    }

    fn process_command(&mut self, cmd: AudioCommand) {
        match cmd {
            AudioCommand::StartRecording => {
                self.loop_len = 0;
                self.loop_write = 0;
                self.loop_read = 0;
                self.loop_recording = true;
                self.loop_overdub = false;
                self.loop_playing = false;
                for sample in &mut self.loop_buf {
                    *sample = 0.0;
                }
            }
            AudioCommand::StartOverdub => {
                self.loop_recording = true;
                self.loop_overdub = true;
                self.loop_playing = true;
            }
            AudioCommand::StopRecording => {
                self.loop_recording = false;
                self.loop_overdub = false;
                if self.loop_len > 0 {
                    self.loop_playing = true;
                    if self.loop_gain < 0.1 {
                        self.loop_gain = 0.7;
                    }
                }
            }
            AudioCommand::SetPlaying(playing) => {
                self.loop_playing = playing;
                if !playing {
                    self.loop_read = 0;
                }
            }
            AudioCommand::ClearLoop => {
                self.loop_len = 0;
                self.loop_write = 0;
                self.loop_read = 0;
                self.loop_recording = false;
                self.loop_overdub = false;
                self.loop_playing = false;
                for sample in &mut self.loop_buf {
                    *sample = 0.0;
                }
            }
            AudioCommand::RestoreLoop { length } => {
                self.loop_len = length;
                self.loop_write = if length > 0 { length } else { 0 };
                self.loop_read = 0;
                self.loop_recording = false;
                self.loop_overdub = false;
                self.loop_playing = length > 0;
            }
            AudioCommand::SetLoopGain(gain) => {
                self.loop_gain = gain;
            }
            AudioCommand::SetLoopSpeed(speed) => {
                self.loop_speed = speed.clamp(0.25, 4.0);
            }
            AudioCommand::TriggerDrum(voice_idx) => {
                if voice_idx < NUM_DRUM_VOICES {
                    self.drum_state[voice_idx].age = Some(0);
                }
            }
            AudioCommand::SnapshotLoop { length: _ } => {
                // No-op on audio side - used by UI for undo snapshots
            }
            AudioCommand::SampleStartRecording => {
                self.sloop_len = 0;
                self.sloop_write = 0;
                self.sloop_read = 0;
                self.sloop_recording = true;
                self.sloop_overdub = false;
                self.sloop_playing = false;
                for sample in &mut self.sloop_buf {
                    *sample = 0.0;
                }
            }
            AudioCommand::SampleStartOverdub => {
                self.sloop_recording = true;
                self.sloop_overdub = true;
                self.sloop_playing = true;
            }
            AudioCommand::SampleStopRecording => {
                self.sloop_recording = false;
                self.sloop_overdub = false;
                if self.sloop_len > 0 {
                    self.sloop_playing = true;
                    if self.sloop_gain < 0.1 {
                        self.sloop_gain = 0.7;
                    }
                }
            }
            AudioCommand::SampleSetPlaying(playing) => {
                self.sloop_playing = playing;
                if !playing {
                    self.sloop_read = 0;
                }
            }
            AudioCommand::SampleClearLoop => {
                self.sloop_len = 0;
                self.sloop_write = 0;
                self.sloop_read = 0;
                self.sloop_recording = false;
                self.sloop_overdub = false;
                self.sloop_playing = false;
                for sample in &mut self.sloop_buf {
                    *sample = 0.0;
                }
            }
            AudioCommand::SampleRestoreLoop { length } => {
                self.sloop_len = length;
                self.sloop_write = if length > 0 { length } else { 0 };
                self.sloop_read = 0;
                self.sloop_recording = false;
                self.sloop_overdub = false;
                self.sloop_playing = length > 0;
            }
            AudioCommand::SampleNoteOn {
                note,
                velocity,
                sample_snapshot,
            } => {
                if let Some(s) = sample_snapshot {
                    if !s.buffer.is_empty() {
                        self.sample_params = s;
                    }
                }
                self.sample_note_on(note, (velocity / 127.0).clamp(0.0, 1.0));
            }
            AudioCommand::SampleNoteOff { note } => {
                for v in &mut self.sample_voices {
                    if v.active && v.note == note {
                        v.releasing = true;
                    }
                }
            }
        }
    }

    fn sample_note_on(&mut self, note: u8, velocity: f32) {
        // Same note still held: keep the existing voice (loops in `render_sample`); do not reset phase.
        for v in &self.sample_voices {
            if v.active && v.note == note && !v.releasing {
                return;
            }
        }
        let vel = velocity.clamp(0.0, 1.0).max(0.004);
        let slot = self
            .sample_voices
            .iter()
            .position(|v| !v.active)
            .unwrap_or(0);
        self.sample_voices[slot] = SampleVoice {
            active: true,
            note,
            vel,
            phase: 0.0,
            env: 0.0,
            releasing: false,
        };
    }

    fn render_sample(&mut self, frames: usize) -> Vec<f32> {
        let s = &self.sample_params;
        let mut out = vec![0.0f32; frames];
        if !s.play_enabled || s.buffer.is_empty() {
            for v in &mut self.sample_voices {
                v.active = false;
            }
            return out;
        }
        let buf = &**s.buffer;
        let n = buf.len();
        if n < 2 {
            return out;
        }
        let t0 = (s.trim_start.clamp(0.0, 0.95) * n as f32) as usize;
        let t1 = (s.trim_end.clamp(0.0, 0.95) * n as f32) as usize;
        let region = n.saturating_sub(t0).saturating_sub(t1).max(1);
        let sr = self.sample_rate.max(1.0);
        let atk = (s.attack.max(0.0005) * sr) as f32;
        let rel = dsp::decay_coeff(s.release.max(0.0005), sr);
        let rel_attack = 1.0 / atk.max(1.0);
        let src_sr = s.sample_rate.max(1) as f32;
        let rate_sr = src_sr / sr;

        for i in 0..frames {
            let mut acc = 0.0f32;
            for voice in &mut self.sample_voices {
                if !voice.active {
                    continue;
                }
                let chrom = 2.0f32.powf(
                    (voice.note as f32 - s.root_midi as f32 + s.pitch_semitones) / 12.0,
                ) as f64;
                let step = s.speed.clamp(0.05, 8.0) as f64 * chrom * rate_sr as f64;
                let mut p = voice.phase;
                // While the note is held (`!releasing`), loop within the trimmed region instead of stopping.
                if p >= region as f64 && !voice.releasing && region > 0 {
                    p %= region as f64;
                    voice.phase = p;
                }
                let idx = t0 as f64 + voice.phase;
                let i0 = idx.floor() as usize;
                let frac = (idx - i0 as f64) as f32;
                let last = (t0 + region).saturating_sub(1).min(n.saturating_sub(2));
                let i0 = i0.min(last);
                let s0 = buf.get(i0).copied().unwrap_or(0.0);
                let s1 = buf.get((i0 + 1).min(n - 1)).copied().unwrap_or(s0);
                let sig = s0 * (1.0 - frac) + s1 * frac;

                if voice.releasing {
                    voice.env *= rel;
                    if voice.env < 0.0008 {
                        voice.active = false;
                    }
                } else {
                    voice.env = (voice.env + rel_attack).min(1.0);
                }

                acc += sig * voice.env * voice.vel * s.gain.clamp(0.0, 4.0);
                voice.phase += step;
            }
            out[i] = acc;
        }
        out
    }

    fn render_into(
        &mut self,
        params: &crate::audio_bridge::AudioParams,
        out: &mut [f32],
    ) -> (Vec<f32>, Vec<f32>) {
        let synth_out = self.render_synth(&params.synth, out.len());
        let loop_out = self.render_loop(&params.looper, &synth_out);
        let sample_live = self.render_sample(out.len());
        let sample_loop_out =
            self.render_sample_loop(&params.sample.sample_loop, &sample_live);
        let drum_out = self.render_drums(&params.drums, out.len());

        let mut synth_samples = vec![0.0f32; out.len()];
        let target_gain = params.synth.volume;
        // Smooth gain changes per-sample to prevent zipper noise (~5ms smoothing)
        let gain_alpha = dsp::attack_coeff(0.005, self.sample_rate);

        for i in 0..out.len() {
            // Smooth gain toward target
            self.current_gain += (target_gain - self.current_gain) * gain_alpha;
            let synth_loop_bus = (synth_out[i] + loop_out[i]) * self.current_gain * SYNTH_BUS_GAIN;
            let sample_bus =
                (sample_live[i] + sample_loop_out[i]) * SAMPLE_BUS_GAIN;
            // Keep scope reactive levels representative of full melodic bus.
            synth_samples[i] = synth_loop_bus + sample_bus;
            // FX (drive/delay/reverb) on synth+loop only — sample/drums stay direct and avoid wash.
            let wet_synth = self.apply_fx_sample(synth_loop_bus, &params.synth) + sample_bus;
            let mut mixed = wet_synth + drum_out[i] * DRUM_BUS_GAIN;
            // Continuous soft limiting - applied to ALL samples, no conditional
            mixed = dsp::soft_limit(mixed, LIMIT_DRIVE, LIMIT_CEILING) * OUTPUT_GAIN;
            // FINAL HARD CLAMP - absolutely prevent any sample from exceeding -1.0 to 1.0
            out[i] = mixed.clamp(-0.98, 0.98);
            self.push_scope(out[i]);
        }

        self.update_recording(params.global_recording, out);
        self.sample_clock = self.sample_clock.saturating_add(out.len() as u64);
        
        (synth_samples, drum_out)
    }

    fn render_synth(&mut self, synth: &SynthState, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames];
        let note_active = synth.note_active() || matches!(synth.gate_mode, GateMode::Hold);
        let target_freq = if note_active {
            synth.current_play_freq()
        } else {
            self.freq_current
        };

        // Update envelope coefficients if they changed
        self.envelope.set_times(synth.attack, synth.release, self.sample_rate);
        
        // Sample-rate-aware frequency smoothing
        let freq_alpha = dsp::attack_coeff(0.004, self.sample_rate);

        for sample in &mut out {
            // Use smooth envelope that continues from current value
            // This prevents clicks when retriggering during attack/release
            self.env = self.envelope.process(note_active);
            
            // Smooth frequency changes
            self.freq_current += (target_freq - self.freq_current) * freq_alpha;

            let lfo = match synth.lfo_wave {
                crate::state::synth::LfoWaveform::Sine => (2.0 * PI * self.lfo_phase).sin(),
                crate::state::synth::LfoWaveform::Triangle => {
                    2.0 * (2.0 * (self.lfo_phase - (self.lfo_phase + 0.5).floor())).abs() - 1.0
                }
                crate::state::synth::LfoWaveform::Square => {
                    if self.lfo_phase < 0.5 { 1.0 } else { -1.0 }
                }
            };
            self.lfo_phase = (self.lfo_phase + synth.lfo_rate / self.sample_rate).fract();

            let mut value = 0.0;
            for (osc_idx, osc) in synth.oscillators.iter().enumerate() {
                if osc.level <= 0.0001 {
                    continue;
                }
                let base_ratio = 2.0f32.powf(osc.octave as f32) * 2.0f32.powf(osc.detune_cents / 1200.0);
                let modulated = match synth.lfo_target {
                    crate::state::synth::LfoTarget::Pitch => {
                        self.freq_current * 2.0f32.powf(lfo * synth.lfo_depth * 0.2)
                    }
                    _ => self.freq_current,
                };
                let osc_val = self.osc_voice_mix(osc_idx, osc, modulated * base_ratio, synth.voices);
                value += osc_val * osc.level;
            }

            value *= self.env;
            if matches!(synth.lfo_target, crate::state::synth::LfoTarget::Volume) {
                value *= 1.0 - synth.lfo_depth * 0.5 + (0.5 + 0.5 * lfo) * synth.lfo_depth;
            }

            if synth.filter_on {
                let target_cutoff = if matches!(synth.lfo_target, crate::state::synth::LfoTarget::Filter) {
                    (synth.cutoff + lfo * synth.lfo_depth * 0.4).clamp(0.02, 0.98)
                } else {
                    synth.cutoff.clamp(0.02, 0.98)
                };
                // Smooth cutoff changes to prevent zipper noise (~5ms smoothing)
                let cutoff_alpha = dsp::attack_coeff(0.005, self.sample_rate);
                self.current_cutoff += (target_cutoff - self.current_cutoff) * cutoff_alpha;
                
                // Convert normalized 0-1 cutoff to actual frequency (exponential mapping)
                // This gives musically useful range: 20Hz at 0.0, 18kHz at 1.0
                let cutoff_freq = FILTER_MIN_FREQ * (FILTER_MAX_FREQ / FILTER_MIN_FREQ).powf(self.current_cutoff);
                
                // Calculate proper one-pole lowpass coefficient from frequency
                // alpha = 1 - exp(-2π * fc / sr) for accurate coefficient
                let omega = 2.0 * std::f32::consts::PI * cutoff_freq / self.sample_rate;
                let alpha = (1.0 - (-omega).exp()).clamp(0.0001, 0.9999);
                
                self.filter_z1 += (value - self.filter_z1) * alpha;
                value = self.filter_z1;
            }

            self.last_output = value;
            *sample = value;
        }

        out
    }

    fn osc_voice_mix(
        &mut self,
        osc_idx: usize,
        osc: &OscillatorState,
        freq: f32,
        voices: u8,
    ) -> f32 {
        let voices = voices.clamp(1, MAX_VOICES as u8) as usize;
        let center = (voices.saturating_sub(1)) as f32 / 2.0;
        let mut acc = 0.0;
        for vi in 0..voices {
            let offset = if voices == 1 {
                0.0
            } else {
                ((vi as f32 - center) / center.max(1.0)) * 0.18
            };
            let ratio = 2.0f32.powf(offset / 12.0);
            let voice_freq = freq * ratio;
            let dt = voice_freq / self.sample_rate; // Phase increment for PolyBLEP
            let phase = &mut self.osc_phases[osc_idx][vi];
            *phase = (*phase + dt).fract();
            acc += Self::wave_sample(*phase, dt, osc.waveform);
        }
        acc / voices as f32
    }

    /// Generate waveform sample with PolyBLEP anti-aliasing for saw/square.
    /// dt = phase increment per sample (freq / sample_rate)
    fn wave_sample(phase: f32, dt: f32, waveform: Waveform) -> f32 {
        match waveform {
            Waveform::Sine => (2.0 * PI * phase).sin(),
            Waveform::Triangle => 2.0 * (2.0 * (phase - (phase + 0.5).floor())).abs() - 1.0,
            // PolyBLEP anti-aliased square
            Waveform::Square => dsp::square_blep(phase, dt),
            // PolyBLEP anti-aliased saw
            Waveform::Saw => dsp::saw_blep(phase, dt),
        }
    }

    fn render_loop(
        &mut self,
        looper: &crate::audio_bridge::LooperParams,
        synth_out: &[f32],
    ) -> Vec<f32> {
        let mut out = vec![0.0; synth_out.len()];
        
        // Handle clear request (only process when flag transitions true)
        if looper.clear_requested && !self.last_clear_seen {
            self.loop_len = 0;
            self.loop_write = 0;
            self.loop_read = 0;
            self.loop_recording = false;
            self.loop_overdub = false;
            for sample in &mut self.loop_buf {
                *sample = 0.0;
            }
            self.last_clear_seen = true;
            return out;  // Early return after clear
        }
        if !looper.clear_requested {
            self.last_clear_seen = false;  // Reset when flag is lowered
        }
        
        // Detect transitions for buffer management
        let was_rec = self.loop_recording;
        let _was_overdub = self.loop_overdub;
        let now_rec = looper.recording;
        let now_overdub = looper.overdub;
        
        // Handle recording start transition
        if now_rec && !was_rec {
            if !now_overdub {
                // Starting replace recording: clear buffer
                self.loop_len = 0;
                self.loop_write = 0;
                self.loop_read = 0;
                for sample in &mut self.loop_buf {
                    *sample = 0.0;
                }
            }
            // Starting overdub: don't clear, just enable recording at current position
        }
        
        // Handle recording stop transition
        if !now_rec && was_rec && self.loop_len > 0 {
            // Just stopped recording - loop is captured
        }
        
        // Update tracking state
        self.loop_recording = now_rec;
        self.loop_overdub = now_overdub;
        
        // Now process audio based on current params
        if now_rec && !now_overdub {
            // Replace mode: record new content
            for (i, sample) in synth_out.iter().enumerate() {
                if self.loop_write < self.loop_buf.len() {
                    self.loop_buf[self.loop_write] = *sample;
                    self.loop_write += 1;
                    self.loop_len = self.loop_write;
                }
                if i < out.len() {
                    out[i] = 0.0;
                }
            }
        } else if now_rec && now_overdub && self.loop_len > 0 {
            // Overdub mode: layer on top of existing loop
            let gain = looper.play_gain.clamp(0.0, 1.0);
            for (i, sample) in synth_out.iter().enumerate() {
                let idx = self.loop_write % self.loop_len;
                let current = self.loop_buf[idx];
                self.loop_buf[idx] = (current * 0.72 + *sample * 0.45).tanh();
                out[i] = self.loop_buf[self.loop_read % self.loop_len] * gain;
                self.loop_write = (self.loop_write + 1) % self.loop_len;
                self.loop_read = (self.loop_read + 1) % self.loop_len;
            }
        } else if looper.playing && self.loop_len > 0 {
            // Playback mode: use params for gain, speed, and trim
            let gain = looper.play_gain.clamp(0.0, 1.0);
            let speed_factor = looper.playback_speed.clamp(0.25, 4.0);
            
            // Calculate effective loop region after trim
            let trim_start_samples = (looper.trim_start.clamp(0.0, 0.9) * self.loop_len as f32) as usize;
            let trim_end_samples = (looper.trim_end.clamp(0.0, 0.9) * self.loop_len as f32) as usize;
            let effective_start = trim_start_samples;
            let effective_end = self.loop_len.saturating_sub(trim_end_samples);
            let effective_len = effective_end.saturating_sub(effective_start).max(1);
            
            let mut fractional_pos = self.loop_read as f32;
            
            for sample in &mut out {
                // Wrap within the effective (trimmed) region
                let pos_in_region = (fractional_pos as usize) % effective_len;
                let read_idx = effective_start + pos_in_region;
                *sample = self.loop_buf.get(read_idx).copied().unwrap_or(0.0) * gain;
                fractional_pos += speed_factor;
                if fractional_pos >= effective_len as f32 {
                    fractional_pos -= effective_len as f32;
                }
            }
            self.loop_read = (fractional_pos as usize) % effective_len;
        }
        out
    }

    /// Performance loop for the sample instrument: records `sample_live` only (not drums/synth).
    fn render_sample_loop(
        &mut self,
        looper: &crate::audio_bridge::LooperParams,
        sample_live: &[f32],
    ) -> Vec<f32> {
        let mut out = vec![0.0; sample_live.len()];

        if looper.clear_requested && !self.sloop_last_clear_seen {
            self.sloop_len = 0;
            self.sloop_write = 0;
            self.sloop_read = 0;
            self.sloop_recording = false;
            self.sloop_overdub = false;
            for sample in &mut self.sloop_buf {
                *sample = 0.0;
            }
            self.sloop_last_clear_seen = true;
            return out;
        }
        if !looper.clear_requested {
            self.sloop_last_clear_seen = false;
        }

        let was_rec = self.sloop_recording;
        let now_rec = looper.recording;
        let now_overdub = looper.overdub;

        if now_rec && !was_rec {
            if !now_overdub {
                self.sloop_len = 0;
                self.sloop_write = 0;
                self.sloop_read = 0;
                for sample in &mut self.sloop_buf {
                    *sample = 0.0;
                }
            }
        }

        if !now_rec && was_rec && self.sloop_len > 0 {
            // capture complete
        }

        self.sloop_recording = now_rec;
        self.sloop_overdub = now_overdub;

        if now_rec && !now_overdub {
            for (i, sample) in sample_live.iter().enumerate() {
                if self.sloop_write < self.sloop_buf.len() {
                    self.sloop_buf[self.sloop_write] = *sample;
                    self.sloop_write += 1;
                    self.sloop_len = self.sloop_write;
                }
                if i < out.len() {
                    out[i] = 0.0;
                }
            }
        } else if now_rec && now_overdub && self.sloop_len > 0 {
            let gain = looper.play_gain.clamp(0.0, 1.0);
            for (i, sample) in sample_live.iter().enumerate() {
                let idx = self.sloop_write % self.sloop_len;
                let current = self.sloop_buf[idx];
                self.sloop_buf[idx] = (current * 0.72 + *sample * 0.45).tanh();
                out[i] = self.sloop_buf[self.sloop_read % self.sloop_len] * gain;
                self.sloop_write = (self.sloop_write + 1) % self.sloop_len;
                self.sloop_read = (self.sloop_read + 1) % self.sloop_len;
            }
        } else if looper.playing && self.sloop_len > 0 {
            let gain = looper.play_gain.clamp(0.0, 1.0);
            let speed_factor = looper.playback_speed.clamp(0.25, 4.0);
            let trim_start_samples =
                (looper.trim_start.clamp(0.0, 0.9) * self.sloop_len as f32) as usize;
            let trim_end_samples =
                (looper.trim_end.clamp(0.0, 0.9) * self.sloop_len as f32) as usize;
            let effective_start = trim_start_samples;
            let effective_end = self.sloop_len.saturating_sub(trim_end_samples);
            let effective_len = effective_end.saturating_sub(effective_start).max(1);

            let mut fractional_pos = self.sloop_read as f32;
            for sample in &mut out {
                let pos_in_region = (fractional_pos as usize) % effective_len;
                let read_idx = effective_start + pos_in_region;
                *sample = self.sloop_buf.get(read_idx).copied().unwrap_or(0.0) * gain;
                fractional_pos += speed_factor;
                if fractional_pos >= effective_len as f32 {
                    fractional_pos -= effective_len as f32;
                }
            }
            self.sloop_read = (fractional_pos as usize) % effective_len;
        }
        out
    }

    fn render_drums(&mut self, drums: &DrumState, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames];
        let step_samples = self.sample_rate * 60.0 / drums.bpm.max(40.0) / 4.0;

        // Clear trigger flags at start of render
        self.drum_triggered = [false; 6];

        // Process triggers from params (manual/MIDI triggers)
        for voice in DrumVoice::ALL {
            if drums.triggers[voice.index()] {
                let state = &mut self.drum_state[voice.index()];
                // If already playing, setup crossfade from current value
                if state.age.is_some() {
                    state.crossfade_from = state.last_value;
                    state.crossfade_samples = DRUM_CROSSFADE_SAMPLES;
                }
                state.age = Some(0);
                self.drum_triggered[voice.index()] = true;
            }
        }

        // Sync chain state when chain changes
        if drums.chain.len() != self.chain_len {
            self.chain_len = drums.chain.len();
            // Reset chain position if it's out of bounds
            if self.chain_position >= self.chain_len && self.chain_len > 0 {
                self.chain_position = 0;
            }
        }

        // Get the pattern that should be playing
        let playing_pattern = if drums.chain_mode && !drums.chain.is_empty() {
            drums.chain[self.chain_position % drums.chain.len()]
        } else {
            drums.current_pattern
        };
        let playing_steps = &drums.patterns[playing_pattern];

        for sample in &mut out {
            if drums.running {
                self.seq_accum += 1.0;
                if self.seq_accum >= step_samples {
                    self.seq_accum -= step_samples;
                    
                    // Check if pattern is wrapping (last step -> first step)
                    let was_last_step = self.seq_step == NUM_STEPS - 1;
                    
                    // Trigger drums on this step
                    for voice in DrumVoice::ALL {
                        if playing_steps[voice.index()][self.seq_step] {
                            let state = &mut self.drum_state[voice.index()];
                            // If already playing, setup crossfade from current value
                            if state.age.is_some() {
                                state.crossfade_from = state.last_value;
                                state.crossfade_samples = DRUM_CROSSFADE_SAMPLES;
                            }
                            state.age = Some(0);
                            self.drum_triggered[voice.index()] = true;
                        }
                    }
                    
                    self.seq_step = (self.seq_step + 1) % NUM_STEPS;
                    
                    // Advance chain when pattern completes
                    if was_last_step && drums.chain_mode && !drums.chain.is_empty() {
                        self.chain_position = (self.chain_position + 1) % drums.chain.len();
                    }
                }
            }

            let mut mix = 0.0;
            for voice in DrumVoice::ALL {
                let idx = voice.index();
                // First, read state and get the sample (no mutable borrow yet)
                let (age_opt, crossfade_from, crossfade_remaining) = {
                    let s = &self.drum_state[idx];
                    (s.age, s.crossfade_from, s.crossfade_samples)
                };
                
                if let Some(age) = age_opt {
                    // Get sample (needs &mut self for noise)
                    let raw_sample = self.drum_voice_sample(voice, age, drums.bank, idx);
                    
                    // Apply crossfade if retriggering
                    let mut sample_val = if crossfade_remaining > 0 {
                        let t = crossfade_remaining as f32 / DRUM_CROSSFADE_SAMPLES as f32;
                        // Crossfade: old value * t + new value * (1-t)
                        crossfade_from * t + raw_sample * (1.0 - t)
                    } else {
                        raw_sample
                    };

                    let limit = match voice {
                        DrumVoice::Kick => 8_000,
                        DrumVoice::Snare => 6_000,
                        DrumVoice::Clap => 5_500,
                        DrumVoice::HiHat => 2_800,
                        DrumVoice::Tom => 7_000,
                        DrumVoice::Cymbal => 8_500,
                    };
                    let fade_len = DRUM_TAIL_FADE_SAMPLES.min(limit).max(32);
                    let samples_left = limit.saturating_sub(age);
                    if samples_left < fade_len {
                        let u = (fade_len - samples_left) as f32 / fade_len as f32;
                        // Cosine ramp to 0 at cutoff — removes post-hit clip when envelope is still non-zero
                        sample_val *= 0.5 * (1.0 + (PI * u).cos());
                    }

                    // Now update state (mutable borrow)
                    let state = &mut self.drum_state[idx];
                    if crossfade_remaining > 0 {
                        state.crossfade_samples = crossfade_remaining - 1;
                    }
                    state.last_value = sample_val;

                    let vgain = if matches!(voice, DrumVoice::Kick) {
                        DRUM_VOICE_GAIN * KICK_VOICE_GAIN_MUL
                    } else {
                        DRUM_VOICE_GAIN
                    };
                    let voice_level = sample_val * drums.volumes[idx] * vgain;
                    mix += voice_level;

                    let next_age = age + 1;
                    state.age = (next_age < limit).then_some(next_age);
                }
            }
            // Single gentle bus cap (per-voice limiters removed to reduce hashy saturation)
            *sample = dsp::soft_limit(mix, 1.12, 0.98);
        }

        out
    }

    fn drum_voice_sample(&mut self, voice: DrumVoice, age: usize, bank: usize, voice_idx: usize) -> f32 {
        let t = age as f32 / self.sample_rate;
        let noise = self.noise();
        let b = get_bank(bank);
        match voice {
            DrumVoice::Kick => {
                let phase = &mut self.drum_state[voice_idx].kick_phase;
                if age == 0 {
                    *phase = 0.0;
                }
                kick_sample_osc(self.sample_rate, phase, t, noise, &b.kick)
            }
            DrumVoice::Snare => self.snare_sample(t, noise, &b.snare),
            DrumVoice::Clap => self.clap_sample(t, noise, b.clap_decay),
            DrumVoice::HiHat => self.hihat_sample(t, noise, &b.hihat),
            DrumVoice::Tom => self.tom_sample(t, b.tom_base, b.tom_decay),
            DrumVoice::Cymbal => self.cymbal_sample(t, noise, &b.cymbal),
        }
    }

    fn snare_sample(&self, t: f32, noise: f32, p: &crate::state::drums::SnareParams) -> f32 {
        let tone1 = (2.0 * PI * p.tone1 * t).sin();
        let tone2 = (2.0 * PI * p.tone2 * t).sin();
        let tone = tone1 * p.tone_mix + tone2 * (1.0 - p.tone_mix);
        let tone_env = (-t / p.tone_decay).exp();
        let noise_env = (-t / p.noise_decay).exp();
        let hp_coeff = (2.0 * PI * p.hp_freq / self.sample_rate).min(0.99);
        let noise_hp = noise * hp_coeff;
        let tone_part = tone * p.tone_mix * tone_env;
        let noise_part = noise_hp * p.noise_mix * noise_env;
        tone_part + noise_part
    }

    fn clap_sample(&self, t: f32, noise: f32, decay: f32) -> f32 {
        let burst_rate = 35.0;
        let idx = (t * burst_rate) as i32;
        let burst = if idx < 4 {
            if idx % 2 == 0 { 1.0 } else { 0.3 }
        } else {
            1.0
        };
        let env = (-t / decay).exp();
        noise * burst * env
    }

    fn hihat_sample(&self, t: f32, noise: f32, p: &crate::state::drums::HiHatParams) -> f32 {
        let metal: f32 = p.metal_freqs
            .iter()
            .map(|&f| (2.0 * PI * f * t).sin())
            .sum::<f32>() / 6.0;
        let env = (-t / p.decay).exp();
        let hp_coeff = (2.0 * PI * p.hp_freq / self.sample_rate).min(0.99);
        let noise_hp = noise * hp_coeff;
        (noise_hp * p.noise_mix + metal * p.metal_mix) * env
    }

    fn tom_sample(&self, t: f32, base_freq: f32, decay: f32) -> f32 {
        let pitch_env = (-t / 0.08).exp();
        let freq = base_freq + base_freq * 0.4 * pitch_env;
        let env = (-t / decay).exp();
        (2.0 * PI * freq * t).sin() * env
    }

    fn cymbal_sample(&self, t: f32, noise: f32, p: &crate::state::drums::HiHatParams) -> f32 {
        let metal: f32 = p.metal_freqs
            .iter()
            .map(|&f| (2.0 * PI * f * t).sin())
            .sum::<f32>() / 6.0;
        let env = (-t / p.decay).exp();
        let hp_coeff = (2.0 * PI * p.hp_freq / self.sample_rate).min(0.99);
        let noise_hp = noise * hp_coeff;
        (noise_hp * p.noise_mix + metal * p.metal_mix) * env
    }

    fn noise(&mut self) -> f32 {
        self.drum_noise ^= self.drum_noise << 13;
        self.drum_noise ^= self.drum_noise >> 17;
        self.drum_noise ^= self.drum_noise << 5;
        ((self.drum_noise as f32 / u32::MAX as f32) * 2.0) - 1.0
    }

    fn apply_fx_sample(&mut self, sample: f32, synth: &SynthState) -> f32 {
        // Add tiny DC offset to prevent denormals in feedback paths
        let sample = sample + DENORMAL_PREVENTION;
        
        // Drive/saturation - continuous, always applied
        let drive = 1.0 + synth.fx.drive * 8.0;
        let driven = dsp::soft_limit(sample, drive, 1.0);

        // Warmth: gentle lowpass coloration (uses dedicated warmth_z state)
        let warmed = if synth.fx.warmth > 0.001 {
            let warm_lp_coeff = 0.05 + synth.fx.warmth * 0.15;  // More pronounced effect
            let lp_out = dsp::one_pole_lp(driven, &mut self.warmth_z, warm_lp_coeff);
            // Blend between dry and filtered based on warmth amount
            driven * (1.0 - synth.fx.warmth * 0.6) + lp_out * synth.fx.warmth * 0.6
        } else {
            // Decay filter state when not in use
            self.warmth_z *= 0.95;
            driven
        };

        // Air: high-frequency boost via differentiation (uses dedicated air_z state)
        let air_boost = if synth.fx.air > 0.001 {
            let diff = warmed - self.air_z;
            self.air_z = warmed;
            warmed + diff * synth.fx.air * 1.5  // More pronounced high-freq boost
        } else {
            self.air_z = warmed;
            warmed
        };
        
        // Delay with CLAMPED feedback to prevent oscillation
        let delay_out = if synth.fx.delay_mix > 0.001 {
            let delay_samples = ((0.08 + synth.fx.delay_time * 1.92) * self.sample_rate) as usize;
            let delay_samples = delay_samples.min(self.delay_buf.len() - 1);
            let read_idx = (self.delay_idx + self.delay_buf.len() - delay_samples) % self.delay_buf.len();
            let delayed = self.delay_buf[read_idx];
            
            // Clamp feedback to prevent infinite oscillation and denormals
            let feedback = synth.fx.delay_feedback.min(MAX_FEEDBACK);
            self.delay_buf[self.delay_idx] = dsp::soft_limit(
                air_boost + delayed * feedback + DENORMAL_PREVENTION,
                1.2,
                1.0,
            );
            self.delay_idx = (self.delay_idx + 1) % self.delay_buf.len();
            
            air_boost * (1.0 - synth.fx.delay_mix) + delayed * synth.fx.delay_mix
        } else {
            air_boost
        };

        // Reverb: proper comb + allpass (Freeverb-style)
        self.reverb.set_params(synth.fx.reverb, synth.fx.reverb * 0.35);
        self.reverb.process(delay_out)
    }

    fn update_recording(&mut self, recording: bool, out: &[f32]) {
        if recording {
            self.rec_buffer.extend_from_slice(out);
        }

        if self.was_recording && !recording {
            self.pending_record_flush = true;
        }
        self.was_recording = recording;
    }

    fn write_wav(&mut self, path: &std::path::Path) -> Result<()> {
        if self.rec_buffer.is_empty() {
            self.rec_buffer.clear();
            return Ok(());
        }
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: self.sample_rate as u32,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(path, spec).context("create wav writer")?;
        for sample in self.rec_buffer.drain(..) {
            let pcm = (sample.clamp(-0.999, 0.999) * i16::MAX as f32) as i16;
            writer.write_sample(pcm)?;
            writer.write_sample(pcm)?;
        }
        writer.finalize()?;
        Ok(())
    }

    fn push_scope(&mut self, sample: f32) {
        if self.scope.len() >= 2048 {
            self.scope.pop_front();
        }
        self.scope.push_back(sample);
    }

    /// Export the loop buffer contents for saving to disk.
    /// Returns None if there's no recorded audio.
    /// Returns Some((samples, length, sample_rate)) if there's recorded audio.
    fn export_loop_data(&self) -> Option<(Vec<f32>, usize, u32)> {
        if self.loop_len == 0 {
            return None;
        }
        let data = self.loop_buf[..self.loop_len].to_vec();
        Some((data, self.loop_len, self.sample_rate as u32))
    }

    fn export_sample_loop_data(&self) -> Option<(Vec<f32>, usize, u32)> {
        if self.sloop_len == 0 {
            return None;
        }
        let data = self.sloop_buf[..self.sloop_len].to_vec();
        Some((data, self.sloop_len, self.sample_rate as u32))
    }

    /// Import loop buffer contents from disk.
    fn import_loop_data(&mut self, samples: &[f32], length: usize) {
        let max_len = self.loop_buf.len();
        let copy_len = samples.len().min(length).min(max_len);
        
        // Clear existing buffer
        for sample in &mut self.loop_buf {
            *sample = 0.0;
        }
        
        // Copy in new samples
        self.loop_buf[..copy_len].copy_from_slice(&samples[..copy_len]);
        self.loop_len = copy_len;
        self.loop_write = copy_len;
        self.loop_read = 0;
        self.loop_playing = copy_len > 0;
        self.loop_recording = false;
        self.loop_overdub = false;
        if self.loop_gain < 0.1 && copy_len > 0 {
            self.loop_gain = 0.7;
        }
    }

    fn import_sample_loop_data(&mut self, samples: &[f32], length: usize) {
        let max_len = self.sloop_buf.len();
        let copy_len = samples.len().min(length).min(max_len);
        for sample in &mut self.sloop_buf {
            *sample = 0.0;
        }
        self.sloop_buf[..copy_len].copy_from_slice(&samples[..copy_len]);
        self.sloop_len = copy_len;
        self.sloop_write = copy_len;
        self.sloop_read = 0;
        self.sloop_playing = copy_len > 0;
        self.sloop_recording = false;
        self.sloop_overdub = false;
        if self.sloop_gain < 0.1 && copy_len > 0 {
            self.sloop_gain = 0.7;
        }
    }
}

fn peak_level(signal: &[f32]) -> f32 {
    signal
        .iter()
        .fold(0.0f32, |acc, value| acc.max(value.abs()))
}

#[cfg(test)]
mod sample_playback_tests {
    use std::sync::Arc;

    use super::AudioEngine;
    use crate::audio_bridge::{AudioCommand, AudioParams, SampleParams};

    #[test]
    fn sample_note_on_is_audible_in_mix() {
        let mut eng = AudioEngine::new(48_000.0);
        let buf: Vec<f32> = (0..8192)
            .map(|i| ((i as f32) * 0.02).sin() * 0.35)
            .collect();
        let sp = SampleParams {
            buffer: Arc::new(buf),
            sample_rate: 48_000,
            play_enabled: true,
            ..SampleParams::default()
        };
        eng.sample_params = sp.clone();
        eng.process_command(AudioCommand::SampleNoteOn {
            note: 60,
            velocity: 127.0,
            sample_snapshot: Some(sp),
        });
        let mut params = AudioParams::default();
        params.sample = eng.sample_params.clone();
        let mut out = vec![0.0f32; 512];
        eng.render_into(&params, &mut out);
        let peak = out.iter().map(|x| x.abs()).fold(0.0f32, f32::max);
        assert!(peak > 1e-4, "expected non-silent output, peak={peak}");
    }
}
