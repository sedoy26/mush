use crate::{
    commands::AppCommand,
    ports::PortRegistry,
    state::{self, *},
};

pub struct App {
    pub state: AppState,
    pub ports: PortRegistry,
}

impl App {
    pub fn new() -> Self {
        Self {
            state: AppState::default(),
            ports: PortRegistry::default(),
        }
    }

    pub fn with_ports(ports: PortRegistry) -> Self {
        Self {
            state: AppState::default(),
            ports,
        }
    }

    pub fn apply(&mut self, command: AppCommand) {
        match command {
            AppCommand::ToggleHelp => {
                self.state.ui.help_open = !self.state.ui.help_open;
            }
            AppCommand::ToggleSettings => {
                self.state.ui.settings_open = !self.state.ui.settings_open;
            }
            AppCommand::SetSettingsPage(page) => {
                self.state.ui.settings_page = page;
            }
            AppCommand::SetVisualMode(mode) => {
                self.state.ui.visual_mode = mode;
            }
            AppCommand::SetVisualFx(style) => {
                self.state.ui.visual_fx = style;
            }
            AppCommand::SetVisualFxDepth(value) => {
                self.state.ui.visual_fx_depth = value.clamp(0.0, 1.0);
            }
            AppCommand::ToggleScopeDrums => {
                self.state.ui.scope_show_drums = !self.state.ui.scope_show_drums;
            }
            AppCommand::NoteOnKeyboard(offset) => {
                self.state.synth.key_offset = Some(offset);
                self.state.synth.key_note_on = true;
            }
            AppCommand::NoteOffKeyboard => {
                self.state.synth.key_note_on = false;
                self.state.synth.key_offset = None;
            }
            AppCommand::NoteOnMidi(note) => {
                self.state.synth.midi_note = Some(note);
                self.state.synth.midi_note_on = true;
                self.state.midi.push_held(note);
            }
            AppCommand::NoteOffMidi(note) => {
                self.state.midi.release_held(note);
                self.state.synth.midi_note = self.state.midi.active_note();
                self.state.synth.midi_note_on = self.state.synth.midi_note.is_some();
            }
            AppCommand::SetBaseMidi(value) => {
                self.state.synth.base_midi = value.clamp(0, 127);
            }
            AppCommand::SetVolume(value) => {
                self.state.synth.volume = value.clamp(0.0, 1.0);
            }
            AppCommand::SetAttack(value) => {
                self.state.synth.attack = value.clamp(0.001, 2.0);
            }
            AppCommand::SetRelease(value) => {
                self.state.synth.release = value.clamp(0.01, 4.0);
            }
            AppCommand::SetGateMode(enabled) => {
                self.state.synth.gate_mode = if enabled {
                    synth::GateMode::Hold
                } else {
                    synth::GateMode::Trigger
                };
            }
            AppCommand::SelectOscillator(index) => {
                if index < self.state.synth.oscillators.len() {
                    self.state.synth.active_osc = index;
                }
            }
            AppCommand::SetWaveform {
                oscillator,
                waveform,
            } => {
                if let Some(osc) = self.state.synth.oscillators.get_mut(oscillator) {
                    osc.waveform = waveform;
                }
            }
            AppCommand::SetOscillatorLevel { oscillator, level } => {
                if let Some(osc) = self.state.synth.oscillators.get_mut(oscillator) {
                    osc.level = level.clamp(0.0, 1.0);
                }
            }
            AppCommand::SetOscillatorOctave { oscillator, octave } => {
                if let Some(osc) = self.state.synth.oscillators.get_mut(oscillator) {
                    osc.octave = octave.clamp(-3, 3);
                }
            }
            AppCommand::SetOscillatorDetune { oscillator, cents } => {
                if let Some(osc) = self.state.synth.oscillators.get_mut(oscillator) {
                    osc.detune_cents = cents.clamp(-100.0, 100.0);
                }
            }
            AppCommand::SetVoices(voices) => {
                self.state.synth.voices = voices.clamp(1, state::MAX_VOICES as u8);
            }
            AppCommand::ToggleFilter => {
                self.state.synth.filter_on = !self.state.synth.filter_on;
            }
            AppCommand::SetCutoff(value) => {
                self.state.synth.cutoff = value.clamp(0.0, 1.0);
            }
            AppCommand::SetResonance(value) => {
                self.state.synth.resonance = value.clamp(0.0, 1.0);
            }
            AppCommand::SetLfoWaveform(waveform) => {
                self.state.synth.lfo_wave = waveform;
            }
            AppCommand::SetLfoRate(value) => {
                self.state.synth.lfo_rate = value.clamp(0.0, 32.0);
            }
            AppCommand::SetLfoDepth(value) => {
                self.state.synth.lfo_depth = value.clamp(0.0, 1.0);
            }
            AppCommand::SetLfoTarget(target) => {
                self.state.synth.lfo_target = target;
            }
            AppCommand::SetFxDrive(value) => self.state.synth.fx.drive = value.clamp(0.0, 1.0),
            AppCommand::SetFxDelayMix(value) => {
                self.state.synth.fx.delay_mix = value.clamp(0.0, 1.0)
            }
            AppCommand::SetFxDelayFeedback(value) => {
                self.state.synth.fx.delay_feedback = value.clamp(0.0, 1.0)
            }
            AppCommand::SetFxDelayTime(value) => {
                self.state.synth.fx.delay_time = value.clamp(0.0, 1.0)
            }
            AppCommand::SetFxWarmth(value) => self.state.synth.fx.warmth = value.clamp(0.0, 1.0),
            AppCommand::SetFxAir(value) => self.state.synth.fx.air = value.clamp(0.0, 1.0),
            AppCommand::SetFxReverb(value) => self.state.synth.fx.reverb = value.clamp(0.0, 1.0),
            AppCommand::ToggleSequencer => {
                self.state.drums.running = !self.state.drums.running;
            }
            AppCommand::SetDrumStep {
                voice,
                step,
                enabled,
            } => {
                self.state.drums.set_step(voice, step, enabled);
            }
            AppCommand::ClearDrumVoice(voice) => {
                self.state.drums.clear_voice(voice);
            }
            AppCommand::ClearAllDrums => {
                self.state.drums.clear_all();
            }
            AppCommand::SetDrumVolume { voice, level } => {
                self.state.drums.set_level(voice, level);
            }
            AppCommand::SetDrumBank(index) => {
                self.state.drums.bank = index;
            }
            AppCommand::SetBpm(value) => {
                self.state.drums.bpm = value.clamp(40.0, 300.0);
            }
            AppCommand::StartLoopReplace => {
                self.state.looper.begin_replace();
            }
            AppCommand::StopLoopRecording => {
                self.state.looper.stop_recording();
            }
            AppCommand::StartLoopOverdub => {
                self.state.looper.begin_overdub();
            }
            AppCommand::StopLoopOverdub => {
                self.state.looper.recording = false;
                self.state.looper.overdub = false;
            }
            AppCommand::ToggleLoopPlayback => {
                if self.state.looper.has_audio {
                    self.state.looper.playing = !self.state.looper.playing;
                }
            }
            AppCommand::UndoLoopOverdub => {
                self.state.looper.undo_last();
            }
            AppCommand::ClearLoop => {
                self.state.looper.clear();
            }
            AppCommand::ToggleGlobalRecording => {
                self.state.audio.global_recording.recording =
                    !self.state.audio.global_recording.recording;
            }
            AppCommand::SetMidiEnabled(enabled) => {
                self.state.midi.enabled = enabled;
            }
            AppCommand::SelectMidiChannel(channel) => {
                self.state.midi.channel = channel;
            }
            AppCommand::SetMidiNoteInput(enabled) => {
                self.state.midi.note_input = enabled;
            }
            AppCommand::SetMidiPadInput(enabled) => {
                self.state.midi.pad_input = enabled;
            }
            AppCommand::SetMidiNoteRemap { src, dst } => {
                self.state.midi.set_remap(src, dst);
            }
            AppCommand::BindMidiTarget { target, value } => {
                self.state.midi.bindings.insert(target, value);
            }
            AppCommand::SetAudioOutput(selection) => {
                self.state.audio.output = selection;
            }
            AppCommand::SetAudioInput(selection) => {
                self.state.audio.input = selection;
            }
            AppCommand::RefreshAudioDevices => {}
            AppCommand::RefreshMidiDevices => {}
            AppCommand::SaveProject(target) => {
                self.state.project.target = Some(target);
            }
            AppCommand::LoadProject(target) => {
                self.state.project.target = Some(target);
            }
        }
    }

    pub fn snapshot_project(&self) -> project::ProjectData {
        self.state.snapshot_project()
    }
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::AppCommand;

    #[test]
    fn keyboard_note_updates_state() {
        let mut app = App::new();
        app.apply(AppCommand::NoteOnKeyboard(7));
        assert_eq!(app.state.synth.current_play_midi(), 67);
        app.apply(AppCommand::NoteOffKeyboard);
        assert_eq!(app.state.synth.current_play_midi(), 60);
        assert!(!app.state.synth.key_note_on);
    }

    #[test]
    fn loop_replace_clears_previous_state() {
        let mut app = App::new();
        app.state.looper.has_audio = true;
        app.state.looper.playing = true;
        app.apply(AppCommand::StartLoopReplace);
        assert!(app.state.looper.recording);
        assert!(!app.state.looper.has_audio);
        assert!(!app.state.looper.playing);
    }
}
