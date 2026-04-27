use serde::{Deserialize, Serialize};

use super::{MAX_OSCILLATORS, MAX_VOICES};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "WaveformHelper")]
pub enum Waveform {
    #[default]
    Sine,
    Triangle,
    Square,
    Saw,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum WaveformHelper {
    Int(u8),
    Str(String),
}

impl TryFrom<WaveformHelper> for Waveform {
    type Error = String;
    fn try_from(v: WaveformHelper) -> Result<Self, Self::Error> {
        match v {
            WaveformHelper::Int(0) => Ok(Waveform::Sine),
            WaveformHelper::Int(1) => Ok(Waveform::Triangle),
            WaveformHelper::Int(2) => Ok(Waveform::Square),
            WaveformHelper::Int(3) => Ok(Waveform::Saw),
            WaveformHelper::Int(i) => Err(format!("invalid waveform index: {}", i)),
            WaveformHelper::Str(s) => match s.as_str() {
                "Sine" => Ok(Waveform::Sine),
                "Triangle" => Ok(Waveform::Triangle),
                "Square" => Ok(Waveform::Square),
                "Saw" => Ok(Waveform::Saw),
                _ => Err(format!("invalid waveform: {}", s)),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "LfoWaveformHelper")]
pub enum LfoWaveform {
    #[default]
    Sine,
    Triangle,
    Square,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LfoWaveformHelper {
    Int(u8),
    Str(String),
}

impl TryFrom<LfoWaveformHelper> for LfoWaveform {
    type Error = String;
    fn try_from(v: LfoWaveformHelper) -> Result<Self, Self::Error> {
        match v {
            LfoWaveformHelper::Int(0) => Ok(LfoWaveform::Sine),
            LfoWaveformHelper::Int(1) => Ok(LfoWaveform::Triangle),
            LfoWaveformHelper::Int(2) => Ok(LfoWaveform::Square),
            LfoWaveformHelper::Int(i) => Err(format!("invalid lfo waveform index: {}", i)),
            LfoWaveformHelper::Str(s) => match s.as_str() {
                "Sine" => Ok(LfoWaveform::Sine),
                "Triangle" => Ok(LfoWaveform::Triangle),
                "Square" => Ok(LfoWaveform::Square),
                _ => Err(format!("invalid lfo waveform: {}", s)),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "LfoTargetHelper")]
pub enum LfoTarget {
    #[default]
    Pitch,
    Volume,
    Filter,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LfoTargetHelper {
    Int(u8),
    Str(String),
}

impl TryFrom<LfoTargetHelper> for LfoTarget {
    type Error = String;
    fn try_from(v: LfoTargetHelper) -> Result<Self, Self::Error> {
        match v {
            LfoTargetHelper::Int(0) => Ok(LfoTarget::Pitch),
            LfoTargetHelper::Int(1) => Ok(LfoTarget::Volume),
            LfoTargetHelper::Int(2) => Ok(LfoTarget::Filter),
            LfoTargetHelper::Int(i) => Err(format!("invalid lfo target index: {}", i)),
            LfoTargetHelper::Str(s) => match s.as_str() {
                "Pitch" => Ok(LfoTarget::Pitch),
                "Volume" => Ok(LfoTarget::Volume),
                "Filter" => Ok(LfoTarget::Filter),
                _ => Err(format!("invalid lfo target: {}", s)),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "GateModeHelper")]
pub enum GateMode {
    #[default]
    Trigger,
    Hold,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum GateModeHelper {
    Int(u8),
    Str(String),
}

impl TryFrom<GateModeHelper> for GateMode {
    type Error = String;
    fn try_from(v: GateModeHelper) -> Result<Self, Self::Error> {
        match v {
            GateModeHelper::Int(0) => Ok(GateMode::Trigger),
            GateModeHelper::Int(1) => Ok(GateMode::Hold),
            GateModeHelper::Int(i) => Err(format!("invalid gate mode index: {}", i)),
            GateModeHelper::Str(s) => match s.as_str() {
                "Trigger" => Ok(GateMode::Trigger),
                "Hold" => Ok(GateMode::Hold),
                _ => Err(format!("invalid gate mode: {}", s)),
            },
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct OscillatorState {
    pub waveform: Waveform,
    pub level: f32,
    pub octave: i8,
    pub detune_cents: f32,
    #[serde(default)]
    pub voice_phases: [f32; MAX_VOICES],
}

impl Default for OscillatorState {
    fn default() -> Self {
        Self {
            waveform: Waveform::Sine,
            level: 0.75,
            octave: 0,
            detune_cents: 0.0,
            voice_phases: [0.0; MAX_VOICES],
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FxState {
    #[serde(alias = "fx_drive")]
    pub drive: f32,
    #[serde(alias = "fx_delay_mix")]
    pub delay_mix: f32,
    #[serde(alias = "fx_delay_feedback")]
    pub delay_feedback: f32,
    #[serde(alias = "fx_delay_time")]
    pub delay_time: f32,
    #[serde(alias = "fx_warmth")]
    pub warmth: f32,
    #[serde(alias = "fx_air")]
    pub air: f32,
    #[serde(alias = "fx_reverb")]
    pub reverb: f32,
}

impl Default for FxState {
    fn default() -> Self {
        Self::bus_defaults()
    }
}

impl FxState {
    /// Baseline FX for a bus (matches `SynthState::default().fx`).
    pub fn bus_defaults() -> Self {
        Self {
            drive: 0.0,
            delay_mix: 0.0,
            delay_feedback: 0.2,
            delay_time: 0.25,
            warmth: 0.0,
            air: 0.0,
            reverb: 0.0,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SynthState {
    pub volume: f32,
    #[serde(default)]
    pub volume_current: f32,
    pub base_midi: i16,
    #[serde(default)]
    pub key_offset: Option<i8>,
    #[serde(default)]
    pub midi_note: Option<u8>,
    pub attack: f32,
    pub release: f32,
    #[serde(default)]
    pub env: f32,
    pub gate_mode: GateMode,
    #[serde(default)]
    pub key_note_on: bool,
    #[serde(default)]
    pub midi_note_on: bool,
    pub lfo_wave: LfoWaveform,
    pub lfo_rate: f32,
    pub lfo_depth: f32,
    #[serde(default)]
    pub lfo_depth_current: f32,
    pub lfo_target: LfoTarget,
    #[serde(default)]
    pub lfo_phase: f32,
    pub filter_on: bool,
    pub cutoff: f32,
    pub resonance: f32,
    pub oscillators: [OscillatorState; MAX_OSCILLATORS],
    pub active_osc: usize,
    pub voices: u8,
    #[serde(default)]
    pub freq_current: f32,
    #[serde(default, flatten)]
    pub fx: FxState,
    #[serde(default)]
    pub filter_mix: f32,
    #[serde(default)]
    pub filt_z: f32,
    #[serde(default)]
    pub filt_z2: f32,
    #[serde(default)]
    pub last_output: f32,
    #[serde(default)]
    pub xruns: u64,
}

impl SynthState {
    pub fn current_play_midi(&self) -> i16 {
        if self.midi_note_on {
            if let Some(note) = self.midi_note {
                return i16::from(note);
            }
        }

        self.base_midi + i16::from(self.key_offset.unwrap_or(0))
    }

    pub fn note_active(&self) -> bool {
        self.key_note_on || self.midi_note_on
    }

    pub fn current_play_freq(&self) -> f32 {
        midi_to_freq(self.current_play_midi())
    }

    pub fn default_oscillators() -> [OscillatorState; MAX_OSCILLATORS] {
        let first = OscillatorState::default();
        let second = OscillatorState {
            waveform: Waveform::Triangle,
            level: 0.0,
            octave: -1,
            detune_cents: -3.0,
            voice_phases: [0.0; MAX_VOICES],
        };
        [first, second]
    }
}

impl Default for SynthState {
    fn default() -> Self {
        Self {
            volume: 0.5,
            volume_current: 0.5,
            base_midi: 60,
            key_offset: None,
            midi_note: None,
            attack: 0.002,
            release: 0.0005,
            env: 0.0,
            gate_mode: GateMode::Trigger,
            key_note_on: false,
            midi_note_on: false,
            lfo_wave: LfoWaveform::Sine,
            lfo_rate: 2.0,
            lfo_depth: 0.0,
            lfo_depth_current: 0.0,
            lfo_target: LfoTarget::Pitch,
            lfo_phase: 0.0,
            filter_on: false,
            cutoff: 0.8,
            resonance: 0.0,
            oscillators: Self::default_oscillators(),
            active_osc: 0,
            voices: 1,
            freq_current: midi_to_freq(60),
            fx: FxState::bus_defaults(),
            filter_mix: 0.0,
            filt_z: 0.0,
            filt_z2: 0.0,
            last_output: 0.0,
            xruns: 0,
        }
    }
}

pub fn midi_to_freq(midi: i16) -> f32 {
    440.0 * (2.0_f32).powf((f32::from(midi) - 69.0) / 12.0)
}
