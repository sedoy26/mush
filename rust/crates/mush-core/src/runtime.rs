//! Runtime orchestrator with lock-free audio.
//!
//! The Runtime manages the application lifecycle:
//! - Owns AppState (for UI thread mutations)
//! - Owns AudioBridge (for lock-free UI↔Audio communication)
//! - Owns command queue Producer (Consumer is in audio callback)
//! - Periodically syncs state between UI and audio

use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait};
use parking_lot::Mutex;

use crate::{
    audio_bridge::{AudioBridge, AudioCommand, create_command_channel},
    audio::{AudioRuntime, SharedAudio},
    camera::CameraRuntime,
    project_io,
    state::{audio::AudioDeviceInfo, midi::MidiDeviceInfo, AppState},
};

pub struct Runtime {
    /// Application state (owned by UI thread)
    pub state: Arc<Mutex<AppState>>,
    /// Lock-free bridge to audio thread
    pub bridge: Arc<AudioBridge>,
    /// Command queue producer (consumer is in audio callback)
    /// Wrapped in Mutex for interior mutability (UI thread only, no contention)
    command_tx: Mutex<Option<rtrb::Producer<AudioCommand>>>,
    /// Command queue consumer (moved to audio on start)
    command_rx: Mutex<Option<rtrb::Consumer<AudioCommand>>>,
    base_dir: PathBuf,
    audio: SharedAudio,
    audio_runtime: Option<AudioRuntime>,
    _midi_conn: Option<midir::MidiInputConnection<()>>,
    camera: Option<CameraRuntime>,
}

impl Runtime {
    pub fn new(base_dir: PathBuf) -> Result<Self> {
        project_io::ensure_runtime_dirs(&base_dir)?;
        let state = Arc::new(Mutex::new(AppState::default()));
        let bridge = Arc::new(AudioBridge::new());
        let audio = SharedAudio::new(Arc::clone(&bridge), &base_dir);
        let (command_tx, command_rx) = create_command_channel();
        
        let mut runtime = Self {
            state,
            bridge,
            command_tx: Mutex::new(Some(command_tx)),
            command_rx: Mutex::new(Some(command_rx)),
            base_dir,
            audio,
            audio_runtime: None,
            _midi_conn: None,
            camera: None,
        };
        runtime.refresh_audio_devices();
        runtime.refresh_midi_devices();
        Ok(runtime)
    }

    pub fn start_audio(&mut self) -> Result<()> {
        // Take the command consumer - it's moved into the audio callback
        let command_rx = self.command_rx.lock().take()
            .context("audio already started or command channel not initialized")?;
        
        // Push initial params to audio thread
        let selection = {
            let state = self.state.lock();
            self.bridge.update_params(
                &state.synth,
                &state.drums,
                &state.looper,
                state.audio.global_recording.recording,
            );
            state.audio.output.clone()
        };
        
        self.audio_runtime = Some(self.audio.start(&selection, command_rx)?);
        
        let mut state = self.state.lock();
        state.audio.status = "Audio active".to_string();
        Ok(())
    }

    pub fn restart_audio(&mut self) -> Result<()> {
        // Recreate command channel for new audio stream
        let (command_tx, command_rx) = create_command_channel();
        *self.command_tx.lock() = Some(command_tx);
        *self.command_rx.lock() = Some(command_rx);
        self.audio_runtime = None;
        self.start_audio()
    }

    /// Send a command to the audio thread (lock-free push to SPSC queue).
    fn send_command(&self, cmd: AudioCommand) {
        if let Some(ref mut tx) = *self.command_tx.lock() {
            // Try to push - if queue is full, command is dropped (rare)
            let _ = tx.push(cmd);
        }
    }

    /// Call this every UI frame to sync state with audio thread.
    /// This is the main synchronization point.
    pub fn sync_audio(&mut self) {
        // 1. Push current params to audio (lock-free on audio side)
        {
            let mut state = self.state.lock();
            self.bridge.update_params(
                &state.synth,
                &state.drums,
                &state.looper,
                state.audio.global_recording.recording,
            );
            // Reset clear_requested after sending so it's only active for one sync cycle
            state.looper.clear_requested = false;
        }
        
        // 2. Pull reactive state from audio (lock-free reads)
        {
            let mut state = self.state.lock();
            self.bridge.sync_to_app_state(&mut state);
        }
    }

    /// Send a loop command to audio thread (lock-free SPSC push).
    pub fn send_loop_command(&self, cmd: AudioCommand) {
        self.send_command(cmd);
    }

    /// Start recording a new loop (replace existing).
    pub fn start_loop_recording(&self) {
        let mut state = self.state.lock();
        state.looper.clear();
        state.looper.recording = true;
        self.send_command(AudioCommand::StartRecording);
    }

    /// Start overdubbing onto existing loop.
    pub fn start_loop_overdub(&self) {
        let state = self.state.lock();
        if state.looper.has_audio {
            drop(state);
            let mut state = self.state.lock();
            // Snapshot for undo
            let length = state.looper.length;
            state.looper.undo_stack.push(crate::state::looper::LoopSnapshot { length });
            state.looper.recording = true;
            state.looper.playing = true;
            state.looper.overdub = true;
            self.send_command(AudioCommand::StartOverdub);
        }
    }

    /// Stop recording.
    pub fn stop_loop_recording(&self) {
        let mut state = self.state.lock();
        state.looper.recording = false;
        state.looper.overdub = false;
        if state.looper.length > 0 {
            state.looper.playing = true;
            state.looper.play_gain = 0.7;
        }
        self.send_command(AudioCommand::StopRecording);
    }

    /// Toggle loop playback.
    pub fn toggle_loop_playback(&self) {
        let mut state = self.state.lock();
        if state.looper.has_audio {
            state.looper.playing = !state.looper.playing;
            self.send_command(AudioCommand::SetPlaying(state.looper.playing));
        }
    }

    /// Clear loop.
    pub fn clear_loop(&self) {
        let mut state = self.state.lock();
        state.looper.clear();
        self.send_command(AudioCommand::ClearLoop);
    }

    /// Undo last overdub.
    pub fn undo_loop(&self) {
        let mut state = self.state.lock();
        if let Some(snapshot) = state.looper.undo_stack.pop() {
            state.looper.length = snapshot.length;
            state.looper.has_audio = snapshot.length > 0;
            state.looper.playing = state.looper.has_audio;
            state.looper.recording = false;
            state.looper.overdub = false;
            if state.looper.play_gain == 0.0 && state.looper.has_audio {
                state.looper.play_gain = 0.7;
            }
            self.send_command(AudioCommand::RestoreLoop { length: snapshot.length });
        }
    }

    pub fn refresh_audio_devices(&mut self) {
        let host = cpal::default_host();
        let output_devices: Vec<AudioDeviceInfo> = host
            .output_devices()
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|device| {
                Some(AudioDeviceInfo {
                    id: None,
                    name: device.description().ok()?.name().to_string(),
                })
            })
            .collect();
        let input_devices: Vec<AudioDeviceInfo> = host
            .input_devices()
            .ok()
            .into_iter()
            .flatten()
            .filter_map(|device| {
                Some(AudioDeviceInfo {
                    id: None,
                    name: device.description().ok()?.name().to_string(),
                })
            })
            .collect();
        let mut state = self.state.lock();
        state.audio.output_devices = output_devices;
        state.audio.input_devices = input_devices;
    }

    pub fn refresh_midi_devices(&mut self) {
        if let Ok(midi) = midir::MidiInput::new("mush-midi") {
            let ports = midi.ports();
            let mut state = self.state.lock();
            state.project.available = project_io::list_projects(&self.base_dir).unwrap_or_default();
            state.midi.status = format!("{} MIDI inputs", ports.len());
            let devices: Vec<MidiDeviceInfo> = ports
                .iter()
                .filter_map(|port| midi.port_name(port).ok())
                .map(|name| MidiDeviceInfo { name })
                .collect();
            state.midi.devices = devices.clone();
            if state.midi.device_name.is_none() {
                state.midi.device_name = devices.first().map(|item| item.name.clone());
            }
        }
    }

    pub fn set_midi_enabled(&mut self, enabled: bool) -> Result<()> {
        if !enabled {
            self._midi_conn = None;
            let mut state = self.state.lock();
            state.midi.enabled = false;
            state.midi.status = "MIDI disabled".to_string();
            return Ok(());
        }

        let mut midi = midir::MidiInput::new("mush-midi")?;
        midi.ignore(midir::Ignore::None);
        let ports = midi.ports();
        let maybe_port = if let Some(name) = self.state.lock().midi.device_name.clone() {
            ports
                .iter()
                .find(|port| midi.port_name(port).ok().as_deref() == Some(name.as_str()))
                .cloned()
        } else {
            ports.first().cloned()
        };

        let port = maybe_port.context("no MIDI input port available")?;
        let port_name = midi.port_name(&port).unwrap_or_else(|_| "MIDI".to_string());
        let state = Arc::clone(&self.state);
        let conn = midi.connect(
            &port,
            "mush-midi-input",
            move |_stamp, message, _| on_midi_message(&state, message),
            (),
        )?;

        self._midi_conn = Some(conn);
        let mut state = self.state.lock();
        state.midi.enabled = true;
        state.midi.device_name = Some(port_name.clone());
        state.midi.status = format!("Listening: {port_name}");
        Ok(())
    }

    pub fn save_project(&self, name: &str) -> Result<PathBuf> {
        let mut state = self.state.lock().clone();
        
        // Export loop audio to WAV file if present
        if let Some((samples, length, sample_rate)) = self.audio.export_loop() {
            let loop_path = project_io::loop_wav_path(&self.base_dir, name);
            project_io::save_loop_wav(&loop_path, &samples[..length], sample_rate)?;
            // Keep loop state flags for reload
            state.looper.length = length;
            state.looper.has_audio = true;
            state.looper.playing = false; // Don't auto-play on load
            state.looper.recording = false;
            state.looper.overdub = false;
            state.looper.read_pos = 0;
            state.looper.write_pos = length;
            state.looper.undo_stack.clear();
        } else {
            // No loop audio - clear state
            state.looper.length = 0;
            state.looper.write_pos = 0;
            state.looper.read_pos = 0;
            state.looper.recording = false;
            state.looper.playing = false;
            state.looper.overdub = false;
            state.looper.has_audio = false;
            state.looper.undo_stack.clear();
            // Delete any old loop WAV file
            let loop_path = project_io::loop_wav_path(&self.base_dir, name);
            let _ = std::fs::remove_file(loop_path); // Ignore errors
        }
        
        project_io::save_project(&self.base_dir, name, &state)
    }

    pub fn load_project(&self, name: &str) -> Result<()> {
        let mut loaded = project_io::load_project(&self.base_dir, name)?;
        
        // Try to load loop WAV if it exists
        let loop_path = project_io::loop_wav_path(&self.base_dir, name);
        if loop_path.exists() {
            match project_io::load_loop_wav(&loop_path) {
                Ok((samples, length)) => {
                    self.audio.import_loop(&samples, length);
                    // Update state to match imported loop
                    loaded.looper.length = length;
                    loaded.looper.has_audio = length > 0;
                    loaded.looper.playing = false; // User can press P to play
                    loaded.looper.recording = false;
                    loaded.looper.overdub = false;
                    loaded.looper.read_pos = 0;
                    loaded.looper.write_pos = length;
                    // Ensure play_gain is audible (old projects may have saved 0.0)
                    if loaded.looper.play_gain < 0.1 {
                        loaded.looper.play_gain = 0.7;
                    }
                }
                Err(_) => {
                    // Failed to load loop - clear state
                    loaded.looper.length = 0;
                    loaded.looper.write_pos = 0;
                    loaded.looper.read_pos = 0;
                    loaded.looper.recording = false;
                    loaded.looper.playing = false;
                    loaded.looper.overdub = false;
                    loaded.looper.has_audio = false;
                }
            }
        } else {
            // No loop file - clear state
            loaded.looper.length = 0;
            loaded.looper.write_pos = 0;
            loaded.looper.read_pos = 0;
            loaded.looper.recording = false;
            loaded.looper.playing = false;
            loaded.looper.overdub = false;
            loaded.looper.has_audio = false;
        }
        loaded.looper.undo_stack.clear();
        
        *self.state.lock() = loaded;
        Ok(())
    }

    pub fn poll_background(&self) -> Result<()> {
        if let Some(path) = self.audio.flush_recording_if_needed()? {
            let mut state = self.state.lock();
            state.audio.global_recording.last_path = Some(path);
            state.audio.global_recording.last_error = None;
        }
        Ok(())
    }

    pub fn ensure_camera(&mut self) {
        if self.camera.is_none() {
            self.camera = Some(CameraRuntime::start());
        }
    }

    pub fn render_camera_ascii(&mut self, width: usize, height: usize) -> Vec<String> {
        self.ensure_camera();
        if let Some(camera) = &self.camera {
            let state = self.state.lock();
            camera.render_ascii(
                width,
                height,
                state.ui.visual_fx,
                state.ui.visual_fx_depth,
                &state.audio.reactive,
            )
        } else {
            vec!["camera unavailable".to_string()]
        }
    }

    pub fn base_dir(&self) -> &PathBuf {
        &self.base_dir
    }
}

fn on_midi_message(state: &Arc<Mutex<AppState>>, message: &[u8]) {
    if message.is_empty() {
        return;
    }

    let status = message[0] & 0xF0;
    let channel = message[0] & 0x0F;
    let data1 = *message.get(1).unwrap_or(&0);
    let data2 = *message.get(2).unwrap_or(&0);

    let mut state = state.lock();
    state.midi.last_message = format!("{:02X?}", message);
    let channel_matches = match state.midi.channel {
        crate::state::midi::MidiChannel::All => true,
        crate::state::midi::MidiChannel::Index(selected) => selected == channel,
    };
    if !channel_matches {
        return;
    }

    match status {
        0x90 if data2 > 0 => {
            if matches!(
                state.midi.learn_mode,
                crate::state::midi::MidiLearnMode::NoteSource
            ) {
                state.midi.note_edit_in = data1;
                state.midi.learn_mode = crate::state::midi::MidiLearnMode::Off;
                state.midi.status = format!("Source note learned: {data1}");
                return;
            }

            if matches!(
                state.midi.learn_mode,
                crate::state::midi::MidiLearnMode::Bind
            ) {
                let target = state.midi.selected_target();
                if target.is_note_binding() {
                    state.midi.bindings.insert(target.clone(), Some(data1));
                    state.midi.learn_mode = crate::state::midi::MidiLearnMode::Off;
                    state.midi.status = format!("Bound {} to note {data1}", target.label());
                    return;
                }
            }

            if state.midi.pad_input {
                let binding = state.midi.bindings.clone();
                if let Some((target, _)) = binding
                    .into_iter()
                    .find(|(target, binding)| target.is_note_binding() && *binding == Some(data1))
                {
                    match target {
                        crate::state::midi::MidiBindingTarget::PadKick => {
                            state.drums.triggers[0] = true
                        }
                        crate::state::midi::MidiBindingTarget::PadSnare => {
                            state.drums.triggers[1] = true
                        }
                        crate::state::midi::MidiBindingTarget::PadClap => {
                            state.drums.triggers[2] = true
                        }
                        crate::state::midi::MidiBindingTarget::PadHiHat => {
                            state.drums.triggers[3] = true
                        }
                        crate::state::midi::MidiBindingTarget::PadTom => {
                            state.drums.triggers[4] = true
                        }
                        crate::state::midi::MidiBindingTarget::PadCymbal => {
                            state.drums.triggers[5] = true
                        }
                        crate::state::midi::MidiBindingTarget::PadLoopPlay => {
                            if state.looper.has_audio {
                                state.looper.playing = !state.looper.playing;
                            }
                        }
                        crate::state::midi::MidiBindingTarget::PadGlobalRecord => {
                            state.audio.global_recording.recording =
                                !state.audio.global_recording.recording;
                        }
                        _ => {}
                    }
                    return;
                }
            }

            if state.midi.note_input {
                state.midi.push_held(data1);
                state.synth.midi_note = state.midi.active_note();
                state.synth.midi_note_on = state.synth.midi_note.is_some();
            }
        }
        0x80 | 0x90 => {
            state.midi.release_held(data1);
            state.synth.midi_note = state.midi.active_note();
            state.synth.midi_note_on = state.synth.midi_note.is_some();
        }
        0xB0 => {
            if matches!(
                state.midi.learn_mode,
                crate::state::midi::MidiLearnMode::Bind
            ) {
                let target = state.midi.selected_target();
                if !target.is_note_binding() {
                    state.midi.bindings.insert(target.clone(), Some(data1));
                    state.midi.learn_mode = crate::state::midi::MidiLearnMode::Off;
                    state.midi.status = format!("Bound {} to CC {data1}", target.label());
                    return;
                }
            }
            let value = data2 as f32 / 127.0;
            for (target, binding) in state.midi.bindings.clone() {
                if !target.is_note_binding() && binding == Some(data1) {
                    match target {
                        crate::state::midi::MidiBindingTarget::KnobVolume => {
                            state.synth.volume = value
                        }
                        crate::state::midi::MidiBindingTarget::KnobCutoff => {
                            state.synth.cutoff = value
                        }
                        crate::state::midi::MidiBindingTarget::KnobResonance => {
                            state.synth.resonance = value
                        }
                        crate::state::midi::MidiBindingTarget::KnobDrive => {
                            state.synth.fx.drive = value
                        }
                        crate::state::midi::MidiBindingTarget::KnobDelayMix => {
                            state.synth.fx.delay_mix = value
                        }
                        crate::state::midi::MidiBindingTarget::KnobDelayFeedback => {
                            state.synth.fx.delay_feedback = value
                        }
                        crate::state::midi::MidiBindingTarget::KnobDelayTime => {
                            state.synth.fx.delay_time = value
                        }
                        crate::state::midi::MidiBindingTarget::KnobWarmth => {
                            state.synth.fx.warmth = value
                        }
                        crate::state::midi::MidiBindingTarget::KnobAir => {
                            state.synth.fx.air = value
                        }
                        crate::state::midi::MidiBindingTarget::KnobReverb => {
                            state.synth.fx.reverb = value
                        }
                        crate::state::midi::MidiBindingTarget::KnobBpm => {
                            state.drums.bpm = 40.0 + value * 260.0
                        }
                        crate::state::midi::MidiBindingTarget::KnobOsc1Level => {
                            state.synth.oscillators[0].level = value
                        }
                        crate::state::midi::MidiBindingTarget::KnobOsc2Level => {
                            state.synth.oscillators[1].level = value
                        }
                        _ => {}
                    }
                }
            }
        }
        _ => {}
    }
}
