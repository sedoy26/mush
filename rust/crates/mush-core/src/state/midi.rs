use std::collections::{HashMap, VecDeque};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize, Deserialize)]
pub enum MidiBindingTarget {
    PadKick,
    PadSnare,
    PadClap,
    PadHiHat,
    PadTom,
    PadCymbal,
    PadLoopPlay,
    PadGlobalRecord,
    KnobVolume,
    KnobCutoff,
    KnobResonance,
    KnobDrive,
    KnobDelayMix,
    KnobDelayFeedback,
    KnobDelayTime,
    KnobWarmth,
    KnobAir,
    KnobReverb,
    KnobBpm,
    KnobOsc1Level,
    KnobOsc2Level,
}

impl MidiBindingTarget {
    pub const ALL: [MidiBindingTarget; 21] = [
        MidiBindingTarget::PadKick,
        MidiBindingTarget::PadSnare,
        MidiBindingTarget::PadClap,
        MidiBindingTarget::PadHiHat,
        MidiBindingTarget::PadTom,
        MidiBindingTarget::PadCymbal,
        MidiBindingTarget::PadLoopPlay,
        MidiBindingTarget::PadGlobalRecord,
        MidiBindingTarget::KnobVolume,
        MidiBindingTarget::KnobCutoff,
        MidiBindingTarget::KnobResonance,
        MidiBindingTarget::KnobDrive,
        MidiBindingTarget::KnobDelayMix,
        MidiBindingTarget::KnobDelayFeedback,
        MidiBindingTarget::KnobDelayTime,
        MidiBindingTarget::KnobWarmth,
        MidiBindingTarget::KnobAir,
        MidiBindingTarget::KnobReverb,
        MidiBindingTarget::KnobBpm,
        MidiBindingTarget::KnobOsc1Level,
        MidiBindingTarget::KnobOsc2Level,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            MidiBindingTarget::PadKick => "Pad Kick",
            MidiBindingTarget::PadSnare => "Pad Snare",
            MidiBindingTarget::PadClap => "Pad Clap",
            MidiBindingTarget::PadHiHat => "Pad HiHat",
            MidiBindingTarget::PadTom => "Pad Tom",
            MidiBindingTarget::PadCymbal => "Pad Cym",
            MidiBindingTarget::PadLoopPlay => "Pad Loop",
            MidiBindingTarget::PadGlobalRecord => "Pad Wav",
            MidiBindingTarget::KnobVolume => "Knob Vol",
            MidiBindingTarget::KnobCutoff => "Knob Cut",
            MidiBindingTarget::KnobResonance => "Knob Res",
            MidiBindingTarget::KnobDrive => "Knob Drv",
            MidiBindingTarget::KnobDelayMix => "Knob DlyMix",
            MidiBindingTarget::KnobDelayFeedback => "Knob DlyFbk",
            MidiBindingTarget::KnobDelayTime => "Knob DlyTim",
            MidiBindingTarget::KnobWarmth => "Knob Warm",
            MidiBindingTarget::KnobAir => "Knob Air",
            MidiBindingTarget::KnobReverb => "Knob Rev",
            MidiBindingTarget::KnobBpm => "Knob BPM",
            MidiBindingTarget::KnobOsc1Level => "Knob Osc1",
            MidiBindingTarget::KnobOsc2Level => "Knob Osc2",
        }
    }

    pub fn is_note_binding(&self) -> bool {
        matches!(
            self,
            MidiBindingTarget::PadKick
                | MidiBindingTarget::PadSnare
                | MidiBindingTarget::PadClap
                | MidiBindingTarget::PadHiHat
                | MidiBindingTarget::PadTom
                | MidiBindingTarget::PadCymbal
                | MidiBindingTarget::PadLoopPlay
                | MidiBindingTarget::PadGlobalRecord
        )
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum MidiLearnMode {
    #[default]
    Off,
    NoteSource,
    Bind,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "MidiChannelHelper")]
pub enum MidiChannel {
    #[default]
    All,
    Index(u8),
}

#[derive(Deserialize)]
#[serde(untagged)]
enum MidiChannelHelper {
    Int(i8),
    Enum(MidiChannelEnum),
}

#[derive(Deserialize)]
enum MidiChannelEnum {
    All,
    Index(u8),
}

impl TryFrom<MidiChannelHelper> for MidiChannel {
    type Error = String;
    fn try_from(v: MidiChannelHelper) -> Result<Self, Self::Error> {
        match v {
            MidiChannelHelper::Int(i) if i < 0 => Ok(MidiChannel::All),
            MidiChannelHelper::Int(i) => Ok(MidiChannel::Index(i as u8)),
            MidiChannelHelper::Enum(MidiChannelEnum::All) => Ok(MidiChannel::All),
            MidiChannelHelper::Enum(MidiChannelEnum::Index(i)) => Ok(MidiChannel::Index(i)),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MidiDeviceInfo {
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum MidiMessage {
    NoteOn { channel: u8, note: u8, velocity: u8 },
    NoteOff { channel: u8, note: u8 },
    ControlChange { channel: u8, control: u8, value: u8 },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MidiState {
    pub enabled: bool,
    pub devices: Vec<MidiDeviceInfo>,
    pub device_name: Option<String>,
    pub channel: MidiChannel,
    pub note_input: bool,
    pub pad_input: bool,
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub last_message: String,
    #[serde(default)]
    pub learn_mode: MidiLearnMode,
    #[serde(default)]
    pub learn_target_index: usize,
    #[serde(default)]
    pub note_edit_in: u8,
    #[serde(default)]
    pub note_edit_out: u8,
    pub note_map: HashMap<u8, u8>,
    #[serde(default)]
    pub bindings: HashMap<MidiBindingTarget, Option<u8>>,
    /// Session-only (never written to `.mush`; always empty on load).
    #[serde(skip, default)]
    pub held_notes: HashMap<u8, u8>,
    #[serde(skip, default)]
    pub held_order: VecDeque<u8>,
}

impl MidiState {
    pub fn set_remap(&mut self, src: u8, dst: Option<u8>) {
        match dst {
            Some(note) => {
                self.note_map.insert(src, note);
            }
            None => {
                self.note_map.remove(&src);
            }
        }
    }

    pub fn push_held(&mut self, note: u8) {
        let mapped = self.note_map.get(&note).copied().unwrap_or(note);
        self.held_notes.insert(note, mapped);
        self.held_order.retain(|existing| *existing != note);
        self.held_order.push_back(note);
    }

    pub fn release_held(&mut self, note: u8) {
        self.held_notes.remove(&note);
        self.held_order.retain(|existing| *existing != note);
    }

    pub fn active_note(&self) -> Option<u8> {
        self.held_order
            .back()
            .and_then(|src| self.held_notes.get(src))
            .copied()
    }

    pub fn selected_target(&self) -> MidiBindingTarget {
        MidiBindingTarget::ALL[self.learn_target_index % MidiBindingTarget::ALL.len()].clone()
    }
}

impl Default for MidiState {
    fn default() -> Self {
        Self {
            enabled: false,
            devices: Vec::new(),
            device_name: None,
            channel: MidiChannel::All,
            note_input: true,
            pad_input: true,
            status: String::from("MIDI idle"),
            last_message: String::new(),
            learn_mode: MidiLearnMode::Off,
            learn_target_index: 0,
            note_edit_in: 60,
            note_edit_out: 60,
            note_map: HashMap::new(),
            bindings: HashMap::new(),
            held_notes: HashMap::new(),
            held_order: VecDeque::new(),
        }
    }
}
