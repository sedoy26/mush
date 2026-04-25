use serde::{Deserialize, Serialize};

pub mod audio;
pub mod drums;
pub mod looper;
pub mod midi;
pub mod project;
pub mod sample;
pub mod synth;
pub mod ui;

pub const MAX_OSCILLATORS: usize = 2;
pub const MAX_VOICES: usize = 6;
pub const NUM_DRUM_VOICES: usize = 6;
pub const NUM_STEPS: usize = 32;
pub const NUM_PATTERNS: usize = 8;
pub const MAX_CHAIN_LENGTH: usize = 64;

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AppState {
    pub synth: synth::SynthState,
    pub drums: drums::DrumState,
    pub looper: looper::LoopState,
    pub sample: sample::SampleState,
    pub midi: midi::MidiState,
    pub audio: audio::AudioState,
    pub ui: ui::UiState,
    pub project: project::ProjectState,
}

impl AppState {
    pub fn snapshot_project(&self) -> project::ProjectData {
        project::ProjectData {
            synth: self.synth.clone(),
            drums: self.drums.clone(),
            looper: self.looper.snapshot(),
            sample: self.sample.clone(),
            midi: self.midi.clone(),
            audio: self.audio.clone(),
            ui: self.ui.clone(),
        }
    }
}
