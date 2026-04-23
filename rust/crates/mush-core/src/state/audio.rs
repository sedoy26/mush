use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "AudioDeviceSelectionHelper")]
pub enum AudioDeviceSelection {
    DefaultSystem,
    Named(String),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum AudioDeviceSelectionHelper {
    String(String),
    Enum(AudioDeviceSelectionEnum),
}

#[derive(Deserialize)]
enum AudioDeviceSelectionEnum {
    DefaultSystem,
    Named(String),
}

impl TryFrom<AudioDeviceSelectionHelper> for AudioDeviceSelection {
    type Error = String;
    fn try_from(v: AudioDeviceSelectionHelper) -> Result<Self, Self::Error> {
        match v {
            AudioDeviceSelectionHelper::String(s) if s.starts_with("__DEFAULT") => {
                Ok(AudioDeviceSelection::DefaultSystem)
            }
            AudioDeviceSelectionHelper::String(s) => Ok(AudioDeviceSelection::Named(s)),
            AudioDeviceSelectionHelper::Enum(AudioDeviceSelectionEnum::DefaultSystem) => {
                Ok(AudioDeviceSelection::DefaultSystem)
            }
            AudioDeviceSelectionHelper::Enum(AudioDeviceSelectionEnum::Named(s)) => {
                Ok(AudioDeviceSelection::Named(s))
            }
        }
    }
}

impl Default for AudioDeviceSelection {
    fn default() -> Self {
        Self::DefaultSystem
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct AudioDeviceInfo {
    pub id: Option<usize>,
    pub name: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GlobalRecordingState {
    pub recording: bool,
    pub last_path: Option<String>,
    pub last_error: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ReactiveLevels {
    pub master: f32,
    pub synth: f32,
    pub drums: f32,
    pub kick: f32,
    pub snare: f32,
    pub hat: f32,
    pub note: f32,
    pub env: f32,        // Envelope value for camera effects
    pub lfo_phase: f32,  // LFO phase for camera effects
    pub freq: f32,       // Current frequency for camera effects
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioState {
    #[serde(alias = "input_name")]
    pub input: AudioDeviceSelection,
    #[serde(alias = "output_name")]
    pub output: AudioDeviceSelection,
    #[serde(default)]
    pub input_devices: Vec<AudioDeviceInfo>,
    #[serde(default)]
    pub output_devices: Vec<AudioDeviceInfo>,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub xruns: u64,
    #[serde(default)]
    pub global_recording: GlobalRecordingState,
    #[serde(default)]
    pub reactive: ReactiveLevels,
    #[serde(skip, default)]
    pub recent_scope: Vec<f32>,
    #[serde(skip, default)]
    pub recent_scope_synth: Vec<f32>,
}

impl Default for AudioState {
    fn default() -> Self {
        Self {
            input: AudioDeviceSelection::DefaultSystem,
            output: AudioDeviceSelection::DefaultSystem,
            input_devices: Vec::new(),
            output_devices: Vec::new(),
            status: String::from("Audio idle"),
            xruns: 0,
            global_recording: GlobalRecordingState::default(),
            reactive: ReactiveLevels::default(),
            recent_scope: Vec::new(),
            recent_scope_synth: Vec::new(),
        }
    }
}
