use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "ThemeHelper")]
pub enum Theme {
    #[default]
    Magenta,
    Mint,
    Amber,
    Ocean,
    HackerGreen,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ThemeHelper {
    Int(u8),
    Str(String),
}

impl TryFrom<ThemeHelper> for Theme {
    type Error = String;
    fn try_from(v: ThemeHelper) -> Result<Self, Self::Error> {
        match v {
            ThemeHelper::Int(0) => Ok(Theme::Magenta),
            ThemeHelper::Int(1) => Ok(Theme::Mint),
            ThemeHelper::Int(2) => Ok(Theme::Amber),
            ThemeHelper::Int(3) => Ok(Theme::Ocean),
            ThemeHelper::Int(4) => Ok(Theme::HackerGreen),
            ThemeHelper::Int(i) => Err(format!("invalid theme index: {}", i)),
            ThemeHelper::Str(s) => match s.as_str() {
                "Magenta" => Ok(Theme::Magenta),
                "Mint" => Ok(Theme::Mint),
                "Amber" => Ok(Theme::Amber),
                "Ocean" => Ok(Theme::Ocean),
                "HackerGreen" => Ok(Theme::HackerGreen),
                _ => Err(format!("invalid theme: {}", s)),
            },
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "VisualModeHelper")]
pub enum VisualMode {
    #[default]
    Scope,
    Camera,
    // New pluggable visual effects
    Plasma,
    Kaleidoscope,
    MatrixRain,
    Fire,
    Tunnel,
    Donut,
    Fireworks,
    Ripples,
    Radio,
    Cube,
}

impl VisualMode {
    pub const ALL: &'static [VisualMode] = &[
        VisualMode::Scope,
        VisualMode::Camera,
        VisualMode::Plasma,
        VisualMode::Kaleidoscope,
        VisualMode::MatrixRain,
        VisualMode::Fire,
        VisualMode::Tunnel,
        VisualMode::Donut,
        VisualMode::Fireworks,
        VisualMode::Ripples,
        VisualMode::Radio,
        VisualMode::Cube,
    ];

    pub fn name(&self) -> &'static str {
        match self {
            VisualMode::Scope => "Scope",
            VisualMode::Camera => "Camera",
            VisualMode::Plasma => "Plasma",
            VisualMode::Kaleidoscope => "Kaleidoscope",
            VisualMode::MatrixRain => "Matrix",
            VisualMode::Fire => "Fire",
            VisualMode::Tunnel => "Tunnel",
            VisualMode::Donut => "Donut",
            VisualMode::Fireworks => "Fireworks",
            VisualMode::Ripples => "Ripples",
            VisualMode::Radio => "Radio",
            VisualMode::Cube => "Cube",
        }
    }

}

#[derive(Deserialize)]
#[serde(untagged)]
enum VisualModeHelper {
    Int(u8),
    Str(String),
}

impl TryFrom<VisualModeHelper> for VisualMode {
    type Error = String;
    fn try_from(v: VisualModeHelper) -> Result<Self, Self::Error> {
        match v {
            VisualModeHelper::Int(i) => {
                VisualMode::ALL.get(i as usize).copied()
                    .ok_or_else(|| format!("invalid visual mode index: {}", i))
            }
            VisualModeHelper::Str(s) => {
                VisualMode::ALL.iter().find(|m| m.name() == s).copied()
                    .ok_or_else(|| format!("invalid visual mode: {}", s))
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "VisualFxHelper")]
pub enum VisualFx {
    #[default]
    Off,
    KickFlash,
    SynthGlow,
    DrumPunch,
    BassScan,
    EdgePulse,
    GlitchShift,
    GatePoster,
    FireStorm,
    IcePulse,
    ChromaSplit,
    MatrixBeat,
    // New effects
    WaveRipple,     // Synth level creates radial ripple from center
    BeatStrobe,     // Master level strobes screen brightness
    HatSparkle,     // Hi-hat triggers random bright pixels
    SnareBurst,     // Snare creates expanding brightness rings
    BassWobble,     // Kick causes horizontal wave distortion
    LfoSweep,       // LFO phase creates scanning brightness
    EnvelopeFade,   // Envelope value controls overall fade
    DrumGrid,       // Different drums affect different grid regions
    FreqShift,      // Pitch frequency shifts contrast
    ComboReact,     // Complex combination of multiple sources
}

#[derive(Deserialize)]
#[serde(untagged)]
enum VisualFxHelper {
    Int(u8),
    Str(String),
}

impl TryFrom<VisualFxHelper> for VisualFx {
    type Error = String;
    fn try_from(v: VisualFxHelper) -> Result<Self, Self::Error> {
        match v {
            VisualFxHelper::Int(i) => {
                // Convert index to enum variant
                match i {
                    0 => Ok(VisualFx::Off),
                    1 => Ok(VisualFx::KickFlash),
                    2 => Ok(VisualFx::SynthGlow),
                    3 => Ok(VisualFx::DrumPunch),
                    4 => Ok(VisualFx::BassScan),
                    5 => Ok(VisualFx::EdgePulse),
                    6 => Ok(VisualFx::GlitchShift),
                    7 => Ok(VisualFx::GatePoster),
                    8 => Ok(VisualFx::FireStorm),
                    9 => Ok(VisualFx::IcePulse),
                    10 => Ok(VisualFx::ChromaSplit),
                    11 => Ok(VisualFx::MatrixBeat),
                    12 => Ok(VisualFx::WaveRipple),
                    13 => Ok(VisualFx::BeatStrobe),
                    14 => Ok(VisualFx::HatSparkle),
                    15 => Ok(VisualFx::SnareBurst),
                    16 => Ok(VisualFx::BassWobble),
                    17 => Ok(VisualFx::LfoSweep),
                    18 => Ok(VisualFx::EnvelopeFade),
                    19 => Ok(VisualFx::DrumGrid),
                    20 => Ok(VisualFx::FreqShift),
                    21 => Ok(VisualFx::ComboReact),
                    _ => Err(format!("invalid visual fx index: {}", i)),
                }
            }
            VisualFxHelper::Str(s) => {
                // Match string variant names
                match s.as_str() {
                    "Off" => Ok(VisualFx::Off),
                    "KickFlash" => Ok(VisualFx::KickFlash),
                    "SynthGlow" => Ok(VisualFx::SynthGlow),
                    "DrumPunch" => Ok(VisualFx::DrumPunch),
                    "BassScan" => Ok(VisualFx::BassScan),
                    "EdgePulse" => Ok(VisualFx::EdgePulse),
                    "GlitchShift" => Ok(VisualFx::GlitchShift),
                    "GatePoster" => Ok(VisualFx::GatePoster),
                    "FireStorm" => Ok(VisualFx::FireStorm),
                    "IcePulse" => Ok(VisualFx::IcePulse),
                    "ChromaSplit" => Ok(VisualFx::ChromaSplit),
                    "MatrixBeat" => Ok(VisualFx::MatrixBeat),
                    "WaveRipple" => Ok(VisualFx::WaveRipple),
                    "BeatStrobe" => Ok(VisualFx::BeatStrobe),
                    "HatSparkle" => Ok(VisualFx::HatSparkle),
                    "SnareBurst" => Ok(VisualFx::SnareBurst),
                    "BassWobble" => Ok(VisualFx::BassWobble),
                    "LfoSweep" => Ok(VisualFx::LfoSweep),
                    "EnvelopeFade" => Ok(VisualFx::EnvelopeFade),
                    "DrumGrid" => Ok(VisualFx::DrumGrid),
                    "FreqShift" => Ok(VisualFx::FreqShift),
                    "ComboReact" => Ok(VisualFx::ComboReact),
                    _ => Err(format!("invalid visual fx: {}", s)),
                }
            }
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum SettingsPage {
    #[default]
    Main,
    Visuals,
    Project,
    SoundDevice,
    Midi,
}

/// Which performance layer receives QWERTY note keys and MIDI note-on/off (TAB cycles).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub enum TabFocus {
    #[default]
    Synth,
    Drums,
    Sample,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct UiState {
    #[serde(default)]
    pub help_open: bool,
    #[serde(default)]
    pub settings_open: bool,
    #[serde(default)]
    pub settings_page: SettingsPage,
    pub theme: Theme,
    pub visual_mode: VisualMode,
    #[serde(alias = "camera_style")]
    pub visual_fx: VisualFx,
    #[serde(alias = "camera_reactivity")]
    pub visual_fx_depth: f32,
    /// Donut only: how much the torus major radius grows on kick (VISUALS tab "Kick swell").
    #[serde(default = "default_donut_kick_swell")]
    pub donut_kick_swell: f32,
    /// Cube: extra scale on kick (VISUALS "Kick punch").
    #[serde(default = "default_cube_kick_punch")]
    pub cube_kick_punch: f32,
    /// Cube: counter-rotation on hi-hat (VISUALS "Hat rewind").
    #[serde(default = "default_cube_hat_rewind")]
    pub cube_hat_rewind: f32,
    pub scope_show_drums: bool,
    #[serde(default = "default_auto_update")]
    pub auto_update: bool,
    #[serde(default)]
    pub tab_focus: TabFocus,
}

fn default_donut_kick_swell() -> f32 {
    0.45
}

fn default_cube_kick_punch() -> f32 {
    0.55
}

fn default_cube_hat_rewind() -> f32 {
    0.4
}

fn default_auto_update() -> bool {
    true
}

impl Default for UiState {
    fn default() -> Self {
        Self {
            help_open: true,
            settings_open: false,
            settings_page: SettingsPage::Main,
            theme: Theme::Magenta,
            visual_mode: VisualMode::Scope,
            visual_fx: VisualFx::Off,
            visual_fx_depth: 0.85,
            donut_kick_swell: default_donut_kick_swell(),
            cube_kick_punch: default_cube_kick_punch(),
            cube_hat_rewind: default_cube_hat_rewind(),
            scope_show_drums: true,
            auto_update: default_auto_update(),
            tab_focus: TabFocus::default(),
        }
    }
}
