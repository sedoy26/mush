use serde::{Deserialize, Serialize};

use super::{
    audio::AudioState, drums::DrumState, looper::LoopSnapshot, midi::MidiState, sample::SampleState,
    synth::SynthState, ui::UiState,
};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ProjectTarget {
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectData {
    pub synth: SynthState,
    pub drums: DrumState,
    pub looper: LoopSnapshot,
    pub sample: SampleState,
    pub midi: MidiState,
    pub audio: AudioState,
    pub ui: UiState,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectState {
    pub available: Vec<ProjectTarget>,
    pub target: Option<ProjectTarget>,
    pub status: String,
}
