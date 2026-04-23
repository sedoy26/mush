use crate::state::{
    audio::{AudioDeviceInfo, AudioDeviceSelection},
    midi::{MidiDeviceInfo, MidiMessage},
    project::{ProjectData, ProjectTarget},
};

pub trait AudioBackend {
    fn available_inputs(&self) -> Vec<AudioDeviceInfo>;
    fn available_outputs(&self) -> Vec<AudioDeviceInfo>;
    fn select_input(&mut self, selection: &AudioDeviceSelection) -> Result<(), String>;
    fn select_output(&mut self, selection: &AudioDeviceSelection) -> Result<(), String>;
    fn start(&mut self) -> Result<(), String>;
    fn stop(&mut self) -> Result<(), String>;
}

pub trait MidiBackend {
    fn available_inputs(&self) -> Vec<MidiDeviceInfo>;
    fn open_input(&mut self, name: &str) -> Result<(), String>;
    fn close_input(&mut self) -> Result<(), String>;
    fn poll_messages(&mut self) -> Vec<MidiMessage>;
}

pub trait CameraBackend {
    fn start(&mut self) -> Result<(), String>;
    fn stop(&mut self) -> Result<(), String>;
    fn is_running(&self) -> bool;
}

pub trait ProjectStore {
    fn list(&self) -> Vec<ProjectTarget>;
    fn load(&self, target: &ProjectTarget) -> Result<ProjectData, String>;
    fn save(&self, target: &ProjectTarget, data: &ProjectData) -> Result<(), String>;
}

#[derive(Default)]
pub struct PortRegistry {
    pub audio: Option<Box<dyn AudioBackend>>,
    pub midi: Option<Box<dyn MidiBackend>>,
    pub camera: Option<Box<dyn CameraBackend>>,
    pub projects: Option<Box<dyn ProjectStore>>,
}
