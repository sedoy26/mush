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

use std::{collections::VecDeque, f32::consts::PI, path::Path, sync::Arc};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use parking_lot::Mutex;

use crate::{
    audio_bridge::{AudioBridge, AudioCommand},
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
const LIMIT_CEILING: f32 = 0.92;
const LIMIT_DRIVE: f32 = 1.5;  // Soft saturation amount
const OUTPUT_GAIN: f32 = 0.46;
const SYNTH_BUS_GAIN: f32 = 0.68;
const DRUM_BUS_GAIN: f32 = 0.30;

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

    pub fn start(&self, selection: &AudioDeviceSelection, command_rx: rtrb::Consumer<AudioCommand>) -> Result<AudioRuntime> {
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
        
        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_output_stream(
                &stream_config,
                move |data: &mut [f32], _| {
                    let mut rx = command_rx_f32.lock();
                    render_callback(data, &bridge, &engine, &mut rx);
                },
                err_fn,
                None,
            )?,
            cpal::SampleFormat::I16 => {
                let bridge = Arc::clone(&self.bridge);
                let engine = Arc::clone(&self.engine);
                let command_rx_i16 = Arc::clone(&command_rx);
                device.build_output_stream(
                    &stream_config,
                    move |data: &mut [i16], _| {
                        let mut rx = command_rx_i16.lock();
                        render_callback_i16(data, &bridge, &engine, &mut rx);
                    },
                    err_fn,
                    None,
                )?
            }
            cpal::SampleFormat::U16 => {
                let bridge = Arc::clone(&self.bridge);
                let engine = Arc::clone(&self.engine);
                let command_rx_u16 = Arc::clone(&command_rx);
                device.build_output_stream(
                    &stream_config,
                    move |data: &mut [u16], _| {
                        let mut rx = command_rx_u16.lock();
                        render_callback_u16(data, &bridge, &engine, &mut rx);
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
fn render_callback(
    data: &mut [f32],
    bridge: &Arc<AudioBridge>,
    engine: &Arc<Mutex<AudioEngine>>,
    command_rx: &mut rtrb::Consumer<AudioCommand>,
) {
    let channels = 2usize;
    let frames = data.len() / channels;
    let mut mono = vec![0.0f32; frames];

    // Try to get the engine lock - if contended, output silence (rare edge case)
    // This only contends during project save/load (export_loop/import_loop)
    if let Some(mut engine) = engine.try_lock() {
        // Load params from bridge (lock-free via arc-swap)
        let params = bridge.load_params();
        
        // Drain commands from SPSC queue (lock-free, owned by audio thread)
        while let Ok(cmd) = command_rx.pop() {
            engine.process_command(cmd);
        }
        
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
        
        // Also update note activity based on synth envelope
        bridge.reactive.note.store(if engine.env > 0.01 { 1.0 } else { 0.0 });
        
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

    // Write mono to stereo output
    for (frame, sample) in mono.into_iter().enumerate() {
        let base = frame * channels;
        data[base] = sample;
        if channels > 1 {
            data[base + 1] = sample;
        }
    }
}

fn render_callback_i16(
    data: &mut [i16],
    bridge: &Arc<AudioBridge>,
    engine: &Arc<Mutex<AudioEngine>>,
    command_rx: &mut rtrb::Consumer<AudioCommand>,
) {
    let mut scratch = vec![0.0f32; data.len() / 2];
    render_callback_core(&mut scratch, bridge, engine, command_rx);
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
) {
    let mut scratch = vec![0.0f32; data.len() / 2];
    render_callback_core(&mut scratch, bridge, engine, command_rx);
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
) {
    if let Some(mut engine) = engine.try_lock() {
        let params = bridge.load_params();
        // Drain commands from SPSC queue (lock-free)
        while let Ok(cmd) = command_rx.pop() {
            engine.process_command(cmd);
        }
        engine.render_into(&params, mono);
    }
}

struct DrumVoiceState {
    age: Option<usize>,
}

struct AudioEngine {
    sample_rate: f32,
    sample_clock: u64,
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
            last_output: 0.0,
            osc_phases: [[0.0; 6]; 2],
            drum_state: [
                DrumVoiceState { age: None },
                DrumVoiceState { age: None },
                DrumVoiceState { age: None },
                DrumVoiceState { age: None },
                DrumVoiceState { age: None },
                DrumVoiceState { age: None },
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
        }
    }

    fn render_into(
        &mut self,
        params: &crate::audio_bridge::AudioParams,
        out: &mut [f32],
    ) -> (Vec<f32>, Vec<f32>) {
        let synth_out = self.render_synth(&params.synth, out.len());
        let loop_out = self.render_loop(&params.looper, &synth_out);
        let drum_out = self.render_drums(&params.drums, out.len());
        
        let mut synth_samples = vec![0.0f32; out.len()];
        let target_gain = params.synth.volume;
        // Smooth gain changes per-sample to prevent zipper noise (~5ms smoothing)
        let gain_alpha = dsp::attack_coeff(0.005, self.sample_rate);

        for i in 0..out.len() {
            // Smooth gain toward target
            self.current_gain += (target_gain - self.current_gain) * gain_alpha;
            synth_samples[i] = (synth_out[i] + loop_out[i]) * self.current_gain * SYNTH_BUS_GAIN;
            let mut mixed = synth_samples[i] + drum_out[i] * DRUM_BUS_GAIN;
            mixed = self.apply_fx_sample(mixed, &params.synth);
            // Continuous soft limiting - applied to ALL samples, no conditional
            mixed = dsp::soft_limit(mixed, LIMIT_DRIVE, LIMIT_CEILING) * OUTPUT_GAIN;
            out[i] = mixed;
            self.push_scope(mixed);
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
                let alpha = self.current_cutoff * self.current_cutoff;
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

    fn render_drums(&mut self, drums: &DrumState, frames: usize) -> Vec<f32> {
        let mut out = vec![0.0f32; frames];
        let step_samples = self.sample_rate * 60.0 / drums.bpm.max(40.0) / 4.0;

        // Clear trigger flags at start of render
        self.drum_triggered = [false; 6];

        // Process triggers from params (manual/MIDI triggers)
        for voice in DrumVoice::ALL {
            if drums.triggers[voice.index()] {
                self.drum_state[voice.index()].age = Some(0);
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
                            self.drum_state[voice.index()].age = Some(0);
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
                if let Some(age) = self.drum_state[voice.index()].age {
                    let sample_val = self.drum_voice_sample(voice, age, drums.bank);
                    mix += sample_val * drums.volumes[voice.index()] * 0.5;
                    let next_age = age + 1;
                    let limit = match voice {
                        DrumVoice::Kick => 8_000,
                        DrumVoice::Snare => 6_000,
                        DrumVoice::Clap => 5_500,
                        DrumVoice::HiHat => 2_800,
                        DrumVoice::Tom => 7_000,
                        DrumVoice::Cymbal => 8_500,
                    };
                    self.drum_state[voice.index()].age = (next_age < limit).then_some(next_age);
                }
            }
            *sample = mix;
        }

        out
    }

    fn drum_voice_sample(&mut self, voice: DrumVoice, age: usize, bank: usize) -> f32 {
        let t = age as f32 / self.sample_rate;
        let noise = self.noise();
        let b = get_bank(bank);
        match voice {
            DrumVoice::Kick => self.kick_sample(t, noise, &b.kick),
            DrumVoice::Snare => self.snare_sample(t, noise, &b.snare),
            DrumVoice::Clap => self.clap_sample(t, noise, b.clap_decay),
            DrumVoice::HiHat => self.hihat_sample(t, noise, &b.hihat),
            DrumVoice::Tom => self.tom_sample(t, b.tom_base, b.tom_decay),
            DrumVoice::Cymbal => self.cymbal_sample(t, noise, &b.cymbal),
        }
    }

    fn kick_sample(&self, t: f32, noise: f32, p: &crate::state::drums::KickParams) -> f32 {
        let pitch_env = (-t / p.pitch_decay).exp();
        let freq = p.base_freq + (p.sweep_freq - p.base_freq) * pitch_env;
        let phase = 2.0 * PI * freq * t;
        let tone = phase.sin();
        let click_env = (-t / p.click_decay).exp();
        let click = noise * p.click * click_env;
        let amp_env = (-t / p.amp_decay).exp();
        (tone + click) * amp_env
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
}

fn peak_level(signal: &[f32]) -> f32 {
    signal
        .iter()
        .fold(0.0f32, |acc, value| acc.max(value.abs()))
}
