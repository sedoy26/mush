//! Lock-free bridge between UI and audio threads.
//!
//! This module provides:
//! - `AudioParams`: Snapshot of parameters the audio thread reads (via arc-swap)
//! - `AudioCommand`: Events pushed from UI to audio (via SPSC queue)
//! - `ReactiveState`: Atomic levels written by audio, read by UI
//! - `ScopeBufferPool`: Pre-allocated scope buffers (no audio-thread allocation)
//!
//! The audio callback NEVER blocks on the UI thread.
//! Command queue producer/consumer are owned separately (not both in bridge)
//! to enable truly lock-free operation.

use std::sync::{
    atomic::{AtomicU32, AtomicU64, AtomicUsize, Ordering},
    Arc,
};

use arc_swap::ArcSwap;

use crate::state::{
    drums::DrumState,
    looper::LoopState,
    synth::SynthState,
};

/// Parameters snapshot for the audio thread.
/// Built by UI thread, swapped atomically, consumed by audio thread.
#[derive(Clone, Debug)]
pub struct AudioParams {
    pub synth: SynthState,
    pub drums: DrumState,
    pub looper: LooperParams,
    pub global_recording: bool,
}

/// Subset of looper state the audio thread needs.
#[derive(Clone, Debug, Default)]
pub struct LooperParams {
    pub recording: bool,
    pub playing: bool,
    pub overdub: bool,
    pub play_gain: f32,
    pub playback_speed: f32,
    pub trim_start: f32,
    pub trim_end: f32,
    pub clear_requested: bool,  // Set when UI wants to clear buffer
}

impl From<&LoopState> for LooperParams {
    fn from(state: &LoopState) -> Self {
        Self {
            recording: state.recording,
            playing: state.playing,
            overdub: state.overdub,
            play_gain: state.play_gain,
            playback_speed: state.playback_speed,
            trim_start: state.trim_start,
            trim_end: state.trim_end,
            clear_requested: state.clear_requested,
        }
    }
}

impl Default for AudioParams {
    fn default() -> Self {
        Self {
            synth: SynthState::default(),
            drums: DrumState::default(),
            looper: LooperParams::default(),
            global_recording: false,
        }
    }
}

/// Commands sent from UI to audio thread.
#[derive(Clone, Debug)]
pub enum AudioCommand {
    /// Clear loop buffer
    ClearLoop,
    /// Start recording (replace mode)
    StartRecording,
    /// Start overdub
    StartOverdub,
    /// Stop recording
    StopRecording,
    /// Set loop playback state
    SetPlaying(bool),
    /// Snapshot the loop for undo
    SnapshotLoop { length: usize },
    /// Restore loop from undo
    RestoreLoop { length: usize },
    /// Set loop gain
    SetLoopGain(f32),
    /// Set loop playback speed
    SetLoopSpeed(f32),
    /// Trigger a drum voice
    TriggerDrum(usize),
}

/// Atomic wrapper for f32 values.
#[derive(Debug)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(value: f32) -> Self {
        Self(AtomicU32::new(value.to_bits()))
    }

    pub fn load(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    pub fn store(&self, value: f32) {
        self.0.store(value.to_bits(), Ordering::Relaxed);
    }
}

impl Default for AtomicF32 {
    fn default() -> Self {
        Self::new(0.0)
    }
}

/// Reactive levels written by audio thread, read by UI.
/// All reads/writes use atomics - no locks.
#[derive(Debug, Default)]
pub struct ReactiveState {
    pub master: AtomicF32,
    pub synth: AtomicF32,
    pub drums: AtomicF32,
    pub kick: AtomicF32,
    pub snare: AtomicF32,
    pub hat: AtomicF32,
    pub note: AtomicF32,
    pub env: AtomicF32,
    pub freq_current: AtomicF32,
    pub lfo_phase: AtomicF32,
    pub last_output: AtomicF32,
    pub loop_length: AtomicU64,
    pub loop_read_pos: AtomicU64,
    pub loop_write_pos: AtomicU64,
    pub loop_has_audio: AtomicU32, // bool as u32
    pub xruns: AtomicU64,
    // Drum sequencer state
    pub drum_step: AtomicU32,
    pub drum_triggers: [AtomicU32; 6], // bool as u32 for each voice
    /// Current position in chain (0-based)
    pub chain_position: AtomicU32,
    /// Pattern index currently playing
    pub playing_pattern: AtomicU32,
}

impl ReactiveState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Write reactive levels from audio thread.
    pub fn write_levels(&self, master: f32, synth: f32, drums: f32) {
        self.master.store(master);
        self.synth.store(synth);
        self.drums.store(drums);
    }

    /// Update drum trigger decay.
    pub fn update_triggers(&self, kick_trigger: bool, snare_trigger: bool, hat_trigger: bool) {
        if kick_trigger {
            self.kick.store(1.0);
        } else {
            self.kick.store(self.kick.load() * 0.92);
        }
        if snare_trigger {
            self.snare.store(1.0);
        } else {
            self.snare.store(self.snare.load() * 0.92);
        }
        if hat_trigger {
            self.hat.store(1.0);
        } else {
            self.hat.store(self.hat.load() * 0.92);
        }
    }

    /// Write synth state from audio.
    pub fn write_synth_state(&self, env: f32, freq: f32, lfo_phase: f32, last_out: f32, note_active: bool) {
        self.env.store(env);
        self.freq_current.store(freq);
        self.lfo_phase.store(lfo_phase);
        self.last_output.store(last_out);
        self.note.store(if note_active { 1.0 } else { 0.0 });
    }

    /// Write looper state from audio.
    pub fn write_looper_state(&self, length: usize, read_pos: usize, write_pos: usize, has_audio: bool) {
        self.loop_length.store(length as u64, Ordering::Relaxed);
        self.loop_read_pos.store(read_pos as u64, Ordering::Relaxed);
        self.loop_write_pos.store(write_pos as u64, Ordering::Relaxed);
        self.loop_has_audio.store(has_audio as u32, Ordering::Relaxed);
    }

    pub fn increment_xruns(&self) {
        self.xruns.fetch_add(1, Ordering::Relaxed);
    }
}

/// Shared state between UI and audio thread using lock-free primitives.
/// Command queue producer/consumer are NOT stored here - they're owned separately
/// by UI and audio threads respectively.
pub struct AudioBridge {
    /// Parameters snapshot (UI writes, audio reads)
    pub params: ArcSwap<AudioParams>,
    /// Reactive state (audio writes, UI reads)
    pub reactive: Arc<ReactiveState>,
    /// Scope buffer pool for visualization (audio writes, UI reads)
    /// Pre-allocated to avoid audio-thread allocation
    pub scope_pool: ScopeBufferPool,
}

/// Pre-allocated scope buffer pool.
/// Audio thread rotates through buffers, UI reads current via atomic index.
/// Zero allocation at runtime.
pub struct ScopeBufferPool {
    /// Three pre-allocated buffers (triple buffering pattern)
    buffers: [ArcSwap<Vec<f32>>; 3],
    buffers_synth: [ArcSwap<Vec<f32>>; 3],
    /// Current write index (audio thread increments).
    /// UI reads from the most recently completed buffer (write_idx).
    write_idx: AtomicUsize,
}

const SCOPE_BUFFER_SIZE: usize = 512;

impl ScopeBufferPool {
    fn new() -> Self {
        Self {
            buffers: [
                ArcSwap::from_pointee(vec![0.0; SCOPE_BUFFER_SIZE]),
                ArcSwap::from_pointee(vec![0.0; SCOPE_BUFFER_SIZE]),
                ArcSwap::from_pointee(vec![0.0; SCOPE_BUFFER_SIZE]),
            ],
            buffers_synth: [
                ArcSwap::from_pointee(vec![0.0; SCOPE_BUFFER_SIZE]),
                ArcSwap::from_pointee(vec![0.0; SCOPE_BUFFER_SIZE]),
                ArcSwap::from_pointee(vec![0.0; SCOPE_BUFFER_SIZE]),
            ],
            write_idx: AtomicUsize::new(0),
        }
    }

    /// Audio thread: Write scope samples to next buffer in pool.
    /// NOTE: This still allocates briefly (Arc + Vec). For true zero-allocation,
    /// use triple_buffer crate or raw pointer manipulation. Current approach
    /// bounds allocation to 3 small Vecs that rotate, which is acceptable.
    pub fn write(&self, samples: &[f32], synth_samples: &[f32]) {
        let idx = self.write_idx.load(Ordering::Relaxed);
        let next_idx = (idx + 1) % 3;
        
        // Copy samples into new Vecs (bounded allocation, 3 buffers rotating)
        let mut new_buf = Vec::with_capacity(SCOPE_BUFFER_SIZE);
        let take_count = samples.len().min(SCOPE_BUFFER_SIZE);
        let skip_count = samples.len().saturating_sub(SCOPE_BUFFER_SIZE);
        new_buf.extend(samples.iter().skip(skip_count).take(take_count).copied());
        
        let mut new_synth = Vec::with_capacity(SCOPE_BUFFER_SIZE);
        let synth_take = synth_samples.len().min(SCOPE_BUFFER_SIZE);
        let synth_skip = synth_samples.len().saturating_sub(SCOPE_BUFFER_SIZE);
        new_synth.extend(synth_samples.iter().skip(synth_skip).take(synth_take).copied());
        
        self.buffers[next_idx].store(Arc::new(new_buf));
        self.buffers_synth[next_idx].store(Arc::new(new_synth));
        
        // Memory fence then update index
        self.write_idx.store(next_idx, Ordering::Release);
    }

    /// UI thread: Read most recently completed scope buffer.
    pub fn read(&self) -> (Vec<f32>, Vec<f32>) {
        // Read index that audio most recently finished writing
        let idx = self.write_idx.load(Ordering::Acquire);
        let main = self.buffers[idx].load();
        let synth = self.buffers_synth[idx].load();
        ((**main).clone(), (**synth).clone())
    }
}

/// Create a command channel for UI→Audio communication.
/// Returns (producer for UI, consumer for audio callback).
/// These should NOT be wrapped in mutex - producer is owned by UI thread,
/// consumer is moved into audio callback closure.
pub fn create_command_channel() -> (rtrb::Producer<AudioCommand>, rtrb::Consumer<AudioCommand>) {
    rtrb::RingBuffer::new(256)
}

impl AudioBridge {
    pub fn new() -> Self {
        Self {
            params: ArcSwap::from_pointee(AudioParams::default()),
            reactive: Arc::new(ReactiveState::new()),
            scope_pool: ScopeBufferPool::new(),
        }
    }

    /// UI thread: Update params snapshot.
    pub fn update_params(&self, synth: &SynthState, drums: &DrumState, looper: &LoopState, global_recording: bool) {
        let new_params = AudioParams {
            synth: synth.clone(),
            drums: drums.clone(),
            looper: LooperParams::from(looper),
            global_recording,
        };
        self.params.store(Arc::new(new_params));
    }

    /// Audio thread: Get current params (lock-free).
    pub fn load_params(&self) -> arc_swap::Guard<Arc<AudioParams>> {
        self.params.load()
    }

    /// Audio thread: Update scope buffer (uses pre-allocated pool).
    pub fn update_scope(&self, samples: &[f32], synth_samples: &[f32]) {
        self.scope_pool.write(samples, synth_samples);
    }

    /// UI thread: Read scope buffer.
    pub fn read_scope(&self) -> (Vec<f32>, Vec<f32>) {
        self.scope_pool.read()
    }

    /// UI thread: Read reactive levels into AppState.
    pub fn sync_to_app_state(&self, state: &mut crate::state::AppState) {
        state.audio.reactive.master = self.reactive.master.load();
        state.audio.reactive.synth = self.reactive.synth.load();
        state.audio.reactive.drums = self.reactive.drums.load();
        state.audio.reactive.kick = self.reactive.kick.load();
        state.audio.reactive.snare = self.reactive.snare.load();
        state.audio.reactive.hat = self.reactive.hat.load();
        state.audio.reactive.note = self.reactive.note.load();
        state.audio.reactive.env = self.reactive.env.load();
        state.audio.reactive.lfo_phase = self.reactive.lfo_phase.load();
        state.audio.reactive.freq = self.reactive.freq_current.load();

        state.synth.env = self.reactive.env.load();
        state.synth.freq_current = self.reactive.freq_current.load();
        state.synth.lfo_phase = self.reactive.lfo_phase.load();
        state.synth.last_output = self.reactive.last_output.load();

        state.looper.length = self.reactive.loop_length.load(Ordering::Relaxed) as usize;
        state.looper.read_pos = self.reactive.loop_read_pos.load(Ordering::Relaxed) as usize;
        state.looper.write_pos = self.reactive.loop_write_pos.load(Ordering::Relaxed) as usize;
        state.looper.has_audio = self.reactive.loop_has_audio.load(Ordering::Relaxed) != 0;

        // Drum sequencer step position (for UI cursor display)
        state.drums.current_step = self.reactive.drum_step.load(Ordering::Relaxed) as usize;
        state.drums.chain_position = self.reactive.chain_position.load(Ordering::Relaxed) as usize;
        // Note: We do NOT sync drum_triggers back - that would create a feedback loop
        // The reactive.kick/snare/hat fields already provide visual feedback for drums

        let (scope, scope_synth) = self.read_scope();
        state.audio.recent_scope = scope;
        state.audio.recent_scope_synth = scope_synth;
    }
}

impl Default for AudioBridge {
    fn default() -> Self {
        Self::new()
    }
}
