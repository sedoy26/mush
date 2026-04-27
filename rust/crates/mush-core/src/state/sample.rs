//! Mono sample recorded from line-in / mic, trimmed and played chromatically.

use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{looper::LoopState, synth::FxState};

/// Max capture length (seconds) — caps RAM and WAV size.
pub const MAX_SAMPLE_SECONDS: f32 = 45.0;

fn empty_buffer() -> Arc<Vec<f32>> {
    Arc::new(Vec::new())
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SampleState {
    /// Mono PCM at `sample_rate` Hz (sidecar `.sample.wav`; skipped in JSON).
    #[serde(skip, default = "empty_buffer")]
    pub buffer: Arc<Vec<f32>>,
    pub sample_rate: u32,
    /// True while UI is capturing from the input device into `samples`.
    #[serde(skip)]
    pub input_recording: bool,
    /// Trim from start of buffer, 0..~0.95
    pub trim_start: f32,
    /// Trim from end of buffer, 0..~0.95
    pub trim_end: f32,
    /// Linear gain 0..2 applied at playback
    pub gain: f32,
    /// Playback rate through the buffer (before chromatic pitch)
    pub speed: f32,
    /// Semitones added to all notes (fine transpose)
    pub pitch_semitones: f32,
    /// Chromatic anchor (MIDI). Playback rate uses `2^((played_note - root_midi + pitch_semitones)/12)` × `speed`.
    pub root_midi: u8,
    /// Sample playback envelope (seconds)
    pub attack: f32,
    pub release: f32,
    /// When false, keyboard/MIDI do not trigger the sample (synth only).
    pub play_enabled: bool,
    /// Performance loop: records **live sample instrument output** (chromatic voices), same
    /// controls as synth loop (R/T/Y/P/U) when the Sample tab is focused. Saved as `.sampleloop.wav`.
    #[serde(default)]
    pub performance_loop: LoopState,
    /// Per-bus drive / delay / warmth / air / reverb (independent from synth and drums).
    #[serde(default)]
    pub fx: FxState,
}

impl Default for SampleState {
    fn default() -> Self {
        Self {
            buffer: empty_buffer(),
            sample_rate: 48_000,
            input_recording: false,
            trim_start: 0.0,
            trim_end: 0.0,
            gain: 1.0,
            speed: 1.0,
            pitch_semitones: 0.0,
            root_midi: 60,
            attack: 0.003,
            release: 0.08,
            play_enabled: true,
            performance_loop: LoopState::default(),
            fx: FxState::bus_defaults(),
        }
    }
}

impl SampleState {
    pub fn has_audio(&self) -> bool {
        // Short taps / clicks still count as a usable sample for playback.
        self.effective_len() > 16
    }

    /// MIDI note for QWERTY `offset` (semitones) relative to `root_midi`.
    pub fn note_for_keyboard_offset(&self, offset: i8) -> u8 {
        (self.root_midi as i16 + offset as i16).clamp(0, 127) as u8
    }

    /// Usable sample count after trim (in original buffer coordinates).
    pub fn effective_len(&self) -> usize {
        let n = self.buffer.len();
        if n < 2 {
            return 0;
        }
        let a = (self.trim_start.clamp(0.0, 0.95) * n as f32) as usize;
        let b = (self.trim_end.clamp(0.0, 0.95) * n as f32) as usize;
        n.saturating_sub(a).saturating_sub(b).max(0)
    }

    pub fn trim_start_idx(&self) -> usize {
        let n = self.buffer.len();
        (self.trim_start.clamp(0.0, 0.95) * n as f32) as usize
    }

    pub fn trim_end_idx(&self) -> usize {
        let n = self.buffer.len();
        (self.trim_end.clamp(0.0, 0.95) * n as f32) as usize
    }

    pub fn set_buffer(&mut self, samples: Vec<f32>, rate: u32) {
        self.buffer = Arc::new(samples);
        self.sample_rate = rate;
    }

    pub fn clear_buffer(&mut self) {
        self.buffer = empty_buffer();
    }
}
