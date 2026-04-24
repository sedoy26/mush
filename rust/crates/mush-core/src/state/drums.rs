use serde::{Deserialize, Serialize};

use super::{NUM_DRUM_VOICES, NUM_PATTERNS, NUM_STEPS, MAX_CHAIN_LENGTH};

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub enum DrumVoice {
    Kick,
    Snare,
    Clap,
    HiHat,
    Tom,
    Cymbal,
}

impl DrumVoice {
    pub const ALL: [DrumVoice; NUM_DRUM_VOICES] = [
        DrumVoice::Kick,
        DrumVoice::Snare,
        DrumVoice::Clap,
        DrumVoice::HiHat,
        DrumVoice::Tom,
        DrumVoice::Cymbal,
    ];

    pub fn index(self) -> usize {
        match self {
            DrumVoice::Kick => 0,
            DrumVoice::Snare => 1,
            DrumVoice::Clap => 2,
            DrumVoice::HiHat => 3,
            DrumVoice::Tom => 4,
            DrumVoice::Cymbal => 5,
        }
    }
}

/// Parameters for kick drum synthesis
#[derive(Clone, Copy, Debug)]
pub struct KickParams {
    pub base_freq: f32,
    pub sweep_freq: f32,
    pub pitch_decay: f32,
    pub amp_decay: f32,
    pub click: f32,
    pub click_decay: f32,
}

/// Parameters for snare drum synthesis
#[derive(Clone, Copy, Debug)]
pub struct SnareParams {
    pub tone1: f32,
    pub tone2: f32,
    pub tone_mix: f32,
    pub noise_mix: f32,
    pub tone_decay: f32,
    pub noise_decay: f32,
    pub hp_freq: f32,
}

/// Parameters for hi-hat/cymbal synthesis
#[derive(Clone, Copy, Debug)]
pub struct HiHatParams {
    pub decay: f32,
    pub hp_freq: f32,
    pub noise_mix: f32,
    pub metal_mix: f32,
    pub metal_freqs: [f32; 6],
}

/// A complete drum bank with all voice parameters
#[derive(Clone, Debug)]
pub struct DrumBank {
    pub name: &'static str,
    pub kick: KickParams,
    pub snare: SnareParams,
    pub clap_decay: f32,
    pub hihat: HiHatParams,
    pub tom_base: f32,
    pub tom_decay: f32,
    pub cymbal: HiHatParams,
}

pub const NUM_DRUM_BANKS: usize = 20;

pub static DRUM_BANKS: [DrumBank; NUM_DRUM_BANKS] = [
    // 0: TR-808 - Classic analog, deep kick, punchy snare
    DrumBank {
        name: "TR-808",
        kick: KickParams { base_freq: 38.0, sweep_freq: 122.0, pitch_decay: 0.09, amp_decay: 0.48, click: 0.05, click_decay: 0.004 },
        snare: SnareParams { tone1: 185.0, tone2: 330.0, tone_mix: 0.34, noise_mix: 0.66, tone_decay: 0.16, noise_decay: 0.11, hp_freq: 1900.0 },
        clap_decay: 0.08,
        hihat: HiHatParams { decay: 0.045, hp_freq: 9000.0, noise_mix: 0.72, metal_mix: 0.28, metal_freqs: [4020.0, 5220.0, 6460.0, 8120.0, 9300.0, 10500.0] },
        tom_base: 91.0,
        tom_decay: 0.25,
        cymbal: HiHatParams { decay: 0.30, hp_freq: 6400.0, noise_mix: 0.78, metal_mix: 0.22, metal_freqs: [3180.0, 4140.0, 5300.0, 6680.0, 7940.0, 9460.0] },
    },
    // 1: TR-909 - Punchy dance/techno
    DrumBank {
        name: "TR-909",
        kick: KickParams { base_freq: 52.0, sweep_freq: 146.0, pitch_decay: 0.06, amp_decay: 0.34, click: 0.11, click_decay: 0.003 },
        snare: SnareParams { tone1: 228.0, tone2: 342.0, tone_mix: 0.42, noise_mix: 0.58, tone_decay: 0.12, noise_decay: 0.09, hp_freq: 2500.0 },
        clap_decay: 0.07,
        hihat: HiHatParams { decay: 0.035, hp_freq: 9800.0, noise_mix: 0.48, metal_mix: 0.52, metal_freqs: [4180.0, 5480.0, 6420.0, 8360.0, 9340.0, 11020.0] },
        tom_base: 125.0,
        tom_decay: 0.18,
        cymbal: HiHatParams { decay: 0.18, hp_freq: 7200.0, noise_mix: 0.44, metal_mix: 0.56, metal_freqs: [3320.0, 4280.0, 5840.0, 7140.0, 8620.0, 10300.0] },
    },
    // 2: CR-78 - Vintage analog, softer character
    DrumBank {
        name: "CR-78",
        kick: KickParams { base_freq: 64.0, sweep_freq: 82.0, pitch_decay: 0.05, amp_decay: 0.22, click: 0.03, click_decay: 0.003 },
        snare: SnareParams { tone1: 240.0, tone2: 480.0, tone_mix: 0.52, noise_mix: 0.48, tone_decay: 0.10, noise_decay: 0.08, hp_freq: 1800.0 },
        clap_decay: 0.06,
        hihat: HiHatParams { decay: 0.028, hp_freq: 8200.0, noise_mix: 0.84, metal_mix: 0.16, metal_freqs: [3640.0, 4980.0, 6420.0, 7980.0, 9100.0, 10080.0] },
        tom_base: 154.0,
        tom_decay: 0.12,
        cymbal: HiHatParams { decay: 0.14, hp_freq: 5800.0, noise_mix: 0.86, metal_mix: 0.14, metal_freqs: [3020.0, 3940.0, 4820.0, 6020.0, 7360.0, 8920.0] },
    },
    // 3: LinnDrum - Clean digital 80s
    DrumBank {
        name: "LinnDrum",
        kick: KickParams { base_freq: 58.0, sweep_freq: 96.0, pitch_decay: 0.045, amp_decay: 0.24, click: 0.08, click_decay: 0.003 },
        snare: SnareParams { tone1: 198.0, tone2: 286.0, tone_mix: 0.46, noise_mix: 0.54, tone_decay: 0.11, noise_decay: 0.09, hp_freq: 2200.0 },
        clap_decay: 0.07,
        hihat: HiHatParams { decay: 0.038, hp_freq: 9400.0, noise_mix: 0.76, metal_mix: 0.24, metal_freqs: [4280.0, 5260.0, 6120.0, 7360.0, 8640.0, 9560.0] },
        tom_base: 139.0,
        tom_decay: 0.13,
        cymbal: HiHatParams { decay: 0.15, hp_freq: 6800.0, noise_mix: 0.74, metal_mix: 0.26, metal_freqs: [3160.0, 4180.0, 5120.0, 6340.0, 7820.0, 9140.0] },
    },
    // 4: DMX - Hip-hop classic
    DrumBank {
        name: "DMX",
        kick: KickParams { base_freq: 49.0, sweep_freq: 110.0, pitch_decay: 0.05, amp_decay: 0.28, click: 0.09, click_decay: 0.003 },
        snare: SnareParams { tone1: 210.0, tone2: 300.0, tone_mix: 0.40, noise_mix: 0.60, tone_decay: 0.11, noise_decay: 0.09, hp_freq: 2400.0 },
        clap_decay: 0.07,
        hihat: HiHatParams { decay: 0.03, hp_freq: 9800.0, noise_mix: 0.79, metal_mix: 0.21, metal_freqs: [4520.0, 5600.0, 6480.0, 7740.0, 8980.0, 10140.0] },
        tom_base: 118.0,
        tom_decay: 0.15,
        cymbal: HiHatParams { decay: 0.16, hp_freq: 7000.0, noise_mix: 0.78, metal_mix: 0.22, metal_freqs: [3340.0, 4260.0, 5480.0, 6740.0, 8120.0, 9440.0] },
    },
    // 5: DrumTraks - Tight digital
    DrumBank {
        name: "DrumTraks",
        kick: KickParams { base_freq: 54.0, sweep_freq: 92.0, pitch_decay: 0.05, amp_decay: 0.26, click: 0.07, click_decay: 0.003 },
        snare: SnareParams { tone1: 224.0, tone2: 312.0, tone_mix: 0.38, noise_mix: 0.62, tone_decay: 0.10, noise_decay: 0.08, hp_freq: 2600.0 },
        clap_decay: 0.06,
        hihat: HiHatParams { decay: 0.03, hp_freq: 10100.0, noise_mix: 0.70, metal_mix: 0.30, metal_freqs: [4340.0, 5440.0, 6300.0, 7540.0, 8860.0, 10340.0] },
        tom_base: 130.0,
        tom_decay: 0.14,
        cymbal: HiHatParams { decay: 0.14, hp_freq: 7200.0, noise_mix: 0.68, metal_mix: 0.32, metal_freqs: [3240.0, 4320.0, 5480.0, 6740.0, 8260.0, 9680.0] },
    },
    // 6: Simmons - Electronic toms, pitch sweep
    DrumBank {
        name: "Simmons",
        kick: KickParams { base_freq: 46.0, sweep_freq: 168.0, pitch_decay: 0.08, amp_decay: 0.30, click: 0.02, click_decay: 0.003 },
        snare: SnareParams { tone1: 278.0, tone2: 418.0, tone_mix: 0.66, noise_mix: 0.34, tone_decay: 0.14, noise_decay: 0.08, hp_freq: 2800.0 },
        clap_decay: 0.06,
        hihat: HiHatParams { decay: 0.05, hp_freq: 8800.0, noise_mix: 0.34, metal_mix: 0.66, metal_freqs: [2820.0, 3560.0, 4720.0, 6040.0, 7440.0, 8920.0] },
        tom_base: 110.0,
        tom_decay: 0.16,
        cymbal: HiHatParams { decay: 0.22, hp_freq: 6400.0, noise_mix: 0.30, metal_mix: 0.70, metal_freqs: [2340.0, 3140.0, 4260.0, 5480.0, 6920.0, 8400.0] },
    },
    // 7: RX11 - Yamaha digital clean
    DrumBank {
        name: "RX11",
        kick: KickParams { base_freq: 60.0, sweep_freq: 88.0, pitch_decay: 0.04, amp_decay: 0.22, click: 0.08, click_decay: 0.0025 },
        snare: SnareParams { tone1: 236.0, tone2: 330.0, tone_mix: 0.36, noise_mix: 0.64, tone_decay: 0.09, noise_decay: 0.07, hp_freq: 2900.0 },
        clap_decay: 0.055,
        hihat: HiHatParams { decay: 0.025, hp_freq: 10400.0, noise_mix: 0.74, metal_mix: 0.26, metal_freqs: [4680.0, 5720.0, 6640.0, 7860.0, 9180.0, 10560.0] },
        tom_base: 144.0,
        tom_decay: 0.12,
        cymbal: HiHatParams { decay: 0.12, hp_freq: 7600.0, noise_mix: 0.70, metal_mix: 0.30, metal_freqs: [3460.0, 4460.0, 5640.0, 6920.0, 8420.0, 9840.0] },
    },
    // 8: R-8 - Roland clean digital
    DrumBank {
        name: "R-8",
        kick: KickParams { base_freq: 50.0, sweep_freq: 118.0, pitch_decay: 0.055, amp_decay: 0.28, click: 0.07, click_decay: 0.003 },
        snare: SnareParams { tone1: 212.0, tone2: 324.0, tone_mix: 0.44, noise_mix: 0.56, tone_decay: 0.11, noise_decay: 0.08, hp_freq: 2600.0 },
        clap_decay: 0.065,
        hihat: HiHatParams { decay: 0.036, hp_freq: 9800.0, noise_mix: 0.62, metal_mix: 0.38, metal_freqs: [4180.0, 5160.0, 6280.0, 7580.0, 8980.0, 10440.0] },
        tom_base: 120.0,
        tom_decay: 0.15,
        cymbal: HiHatParams { decay: 0.18, hp_freq: 7200.0, noise_mix: 0.60, metal_mix: 0.40, metal_freqs: [3260.0, 4320.0, 5520.0, 6840.0, 8340.0, 9880.0] },
    },
    // 9: SP12 - E-mu sampling classic
    DrumBank {
        name: "SP-1200",
        kick: KickParams { base_freq: 48.0, sweep_freq: 104.0, pitch_decay: 0.05, amp_decay: 0.30, click: 0.06, click_decay: 0.003 },
        snare: SnareParams { tone1: 204.0, tone2: 296.0, tone_mix: 0.40, noise_mix: 0.60, tone_decay: 0.11, noise_decay: 0.09, hp_freq: 2300.0 },
        clap_decay: 0.072,
        hihat: HiHatParams { decay: 0.032, hp_freq: 9400.0, noise_mix: 0.80, metal_mix: 0.20, metal_freqs: [4120.0, 5180.0, 6200.0, 7420.0, 8640.0, 9780.0] },
        tom_base: 115.0,
        tom_decay: 0.16,
        cymbal: HiHatParams { decay: 0.17, hp_freq: 6900.0, noise_mix: 0.78, metal_mix: 0.22, metal_freqs: [3180.0, 4220.0, 5340.0, 6620.0, 8040.0, 9480.0] },
    },
    // 10: MPC60 - Akai sampler, punchy
    DrumBank {
        name: "MPC60",
        kick: KickParams { base_freq: 44.0, sweep_freq: 130.0, pitch_decay: 0.07, amp_decay: 0.36, click: 0.10, click_decay: 0.004 },
        snare: SnareParams { tone1: 195.0, tone2: 310.0, tone_mix: 0.38, noise_mix: 0.62, tone_decay: 0.13, noise_decay: 0.10, hp_freq: 2100.0 },
        clap_decay: 0.085,
        hihat: HiHatParams { decay: 0.04, hp_freq: 8800.0, noise_mix: 0.75, metal_mix: 0.25, metal_freqs: [3900.0, 5100.0, 6300.0, 7700.0, 9100.0, 10200.0] },
        tom_base: 105.0,
        tom_decay: 0.20,
        cymbal: HiHatParams { decay: 0.25, hp_freq: 6200.0, noise_mix: 0.72, metal_mix: 0.28, metal_freqs: [3100.0, 4000.0, 5200.0, 6500.0, 7800.0, 9200.0] },
    },
    // 11: Acoustic - Natural kit simulation
    DrumBank {
        name: "Acoustic",
        kick: KickParams { base_freq: 55.0, sweep_freq: 75.0, pitch_decay: 0.03, amp_decay: 0.20, click: 0.15, click_decay: 0.002 },
        snare: SnareParams { tone1: 180.0, tone2: 260.0, tone_mix: 0.55, noise_mix: 0.45, tone_decay: 0.08, noise_decay: 0.12, hp_freq: 1600.0 },
        clap_decay: 0.05,
        hihat: HiHatParams { decay: 0.06, hp_freq: 7500.0, noise_mix: 0.55, metal_mix: 0.45, metal_freqs: [3200.0, 4400.0, 5800.0, 7200.0, 8600.0, 10000.0] },
        tom_base: 95.0,
        tom_decay: 0.22,
        cymbal: HiHatParams { decay: 0.45, hp_freq: 5000.0, noise_mix: 0.40, metal_mix: 0.60, metal_freqs: [2800.0, 3600.0, 4800.0, 6200.0, 7600.0, 9000.0] },
    },
    // 12: Industrial - Hard, aggressive
    DrumBank {
        name: "Industrial",
        kick: KickParams { base_freq: 35.0, sweep_freq: 180.0, pitch_decay: 0.12, amp_decay: 0.55, click: 0.18, click_decay: 0.005 },
        snare: SnareParams { tone1: 320.0, tone2: 480.0, tone_mix: 0.25, noise_mix: 0.75, tone_decay: 0.08, noise_decay: 0.15, hp_freq: 3200.0 },
        clap_decay: 0.12,
        hihat: HiHatParams { decay: 0.02, hp_freq: 11000.0, noise_mix: 0.90, metal_mix: 0.10, metal_freqs: [5500.0, 6800.0, 8200.0, 9800.0, 11200.0, 12500.0] },
        tom_base: 85.0,
        tom_decay: 0.30,
        cymbal: HiHatParams { decay: 0.08, hp_freq: 8500.0, noise_mix: 0.88, metal_mix: 0.12, metal_freqs: [4200.0, 5600.0, 7000.0, 8400.0, 9800.0, 11200.0] },
    },
    // 13: Lo-Fi - Gritty, tape-saturated
    DrumBank {
        name: "Lo-Fi",
        kick: KickParams { base_freq: 42.0, sweep_freq: 90.0, pitch_decay: 0.06, amp_decay: 0.32, click: 0.04, click_decay: 0.006 },
        snare: SnareParams { tone1: 165.0, tone2: 240.0, tone_mix: 0.50, noise_mix: 0.50, tone_decay: 0.14, noise_decay: 0.11, hp_freq: 1400.0 },
        clap_decay: 0.09,
        hihat: HiHatParams { decay: 0.05, hp_freq: 6500.0, noise_mix: 0.85, metal_mix: 0.15, metal_freqs: [3000.0, 4200.0, 5400.0, 6600.0, 7800.0, 9000.0] },
        tom_base: 100.0,
        tom_decay: 0.18,
        cymbal: HiHatParams { decay: 0.20, hp_freq: 5500.0, noise_mix: 0.82, metal_mix: 0.18, metal_freqs: [2600.0, 3400.0, 4400.0, 5600.0, 6800.0, 8000.0] },
    },
    // 14: Minimal - Tight, subtle
    DrumBank {
        name: "Minimal",
        kick: KickParams { base_freq: 48.0, sweep_freq: 60.0, pitch_decay: 0.03, amp_decay: 0.18, click: 0.12, click_decay: 0.002 },
        snare: SnareParams { tone1: 250.0, tone2: 350.0, tone_mix: 0.30, noise_mix: 0.70, tone_decay: 0.06, noise_decay: 0.05, hp_freq: 3000.0 },
        clap_decay: 0.04,
        hihat: HiHatParams { decay: 0.018, hp_freq: 12000.0, noise_mix: 0.65, metal_mix: 0.35, metal_freqs: [5000.0, 6200.0, 7400.0, 8800.0, 10200.0, 11600.0] },
        tom_base: 140.0,
        tom_decay: 0.10,
        cymbal: HiHatParams { decay: 0.10, hp_freq: 8000.0, noise_mix: 0.60, metal_mix: 0.40, metal_freqs: [3800.0, 4800.0, 6000.0, 7200.0, 8600.0, 10000.0] },
    },
    // 15: Jungle - Fast, breakbeat-ready
    DrumBank {
        name: "Jungle",
        kick: KickParams { base_freq: 56.0, sweep_freq: 140.0, pitch_decay: 0.04, amp_decay: 0.22, click: 0.14, click_decay: 0.002 },
        snare: SnareParams { tone1: 245.0, tone2: 380.0, tone_mix: 0.35, noise_mix: 0.65, tone_decay: 0.07, noise_decay: 0.06, hp_freq: 2800.0 },
        clap_decay: 0.05,
        hihat: HiHatParams { decay: 0.022, hp_freq: 10500.0, noise_mix: 0.58, metal_mix: 0.42, metal_freqs: [4600.0, 5800.0, 7000.0, 8400.0, 9800.0, 11200.0] },
        tom_base: 135.0,
        tom_decay: 0.11,
        cymbal: HiHatParams { decay: 0.12, hp_freq: 7800.0, noise_mix: 0.55, metal_mix: 0.45, metal_freqs: [3500.0, 4600.0, 5800.0, 7000.0, 8400.0, 9800.0] },
    },
    // 16: Trap - Heavy 808-style sub kick
    DrumBank {
        name: "Trap",
        kick: KickParams { base_freq: 32.0, sweep_freq: 160.0, pitch_decay: 0.15, amp_decay: 0.65, click: 0.08, click_decay: 0.005 },
        snare: SnareParams { tone1: 200.0, tone2: 320.0, tone_mix: 0.28, noise_mix: 0.72, tone_decay: 0.10, noise_decay: 0.12, hp_freq: 2200.0 },
        clap_decay: 0.10,
        hihat: HiHatParams { decay: 0.015, hp_freq: 11500.0, noise_mix: 0.70, metal_mix: 0.30, metal_freqs: [5200.0, 6400.0, 7800.0, 9200.0, 10600.0, 12000.0] },
        tom_base: 75.0,
        tom_decay: 0.35,
        cymbal: HiHatParams { decay: 0.06, hp_freq: 9000.0, noise_mix: 0.75, metal_mix: 0.25, metal_freqs: [4000.0, 5200.0, 6600.0, 8000.0, 9400.0, 10800.0] },
    },
    // 17: Gabber - Hardcore distorted
    DrumBank {
        name: "Gabber",
        kick: KickParams { base_freq: 40.0, sweep_freq: 200.0, pitch_decay: 0.10, amp_decay: 0.40, click: 0.20, click_decay: 0.003 },
        snare: SnareParams { tone1: 280.0, tone2: 420.0, tone_mix: 0.45, noise_mix: 0.55, tone_decay: 0.06, noise_decay: 0.08, hp_freq: 3500.0 },
        clap_decay: 0.055,
        hihat: HiHatParams { decay: 0.025, hp_freq: 10000.0, noise_mix: 0.80, metal_mix: 0.20, metal_freqs: [4800.0, 6000.0, 7200.0, 8600.0, 10000.0, 11400.0] },
        tom_base: 95.0,
        tom_decay: 0.22,
        cymbal: HiHatParams { decay: 0.14, hp_freq: 7500.0, noise_mix: 0.78, metal_mix: 0.22, metal_freqs: [3600.0, 4800.0, 6000.0, 7400.0, 8800.0, 10200.0] },
    },
    // 18: Dub - Deep, spacey reggae
    DrumBank {
        name: "Dub",
        kick: KickParams { base_freq: 45.0, sweep_freq: 85.0, pitch_decay: 0.08, amp_decay: 0.42, click: 0.03, click_decay: 0.004 },
        snare: SnareParams { tone1: 175.0, tone2: 255.0, tone_mix: 0.48, noise_mix: 0.52, tone_decay: 0.15, noise_decay: 0.14, hp_freq: 1700.0 },
        clap_decay: 0.11,
        hihat: HiHatParams { decay: 0.055, hp_freq: 7800.0, noise_mix: 0.78, metal_mix: 0.22, metal_freqs: [3400.0, 4600.0, 5800.0, 7200.0, 8600.0, 10000.0] },
        tom_base: 88.0,
        tom_decay: 0.28,
        cymbal: HiHatParams { decay: 0.35, hp_freq: 5800.0, noise_mix: 0.75, metal_mix: 0.25, metal_freqs: [2900.0, 3800.0, 4900.0, 6200.0, 7500.0, 8900.0] },
    },
    // 19: Funk - Tight, punchy grooves
    DrumBank {
        name: "Funk",
        kick: KickParams { base_freq: 58.0, sweep_freq: 95.0, pitch_decay: 0.04, amp_decay: 0.20, click: 0.12, click_decay: 0.002 },
        snare: SnareParams { tone1: 220.0, tone2: 340.0, tone_mix: 0.52, noise_mix: 0.48, tone_decay: 0.08, noise_decay: 0.07, hp_freq: 2400.0 },
        clap_decay: 0.06,
        hihat: HiHatParams { decay: 0.032, hp_freq: 9200.0, noise_mix: 0.60, metal_mix: 0.40, metal_freqs: [4100.0, 5300.0, 6500.0, 7900.0, 9300.0, 10700.0] },
        tom_base: 130.0,
        tom_decay: 0.14,
        cymbal: HiHatParams { decay: 0.22, hp_freq: 6600.0, noise_mix: 0.58, metal_mix: 0.42, metal_freqs: [3300.0, 4300.0, 5500.0, 6800.0, 8200.0, 9600.0] },
    },
];

pub fn get_bank(index: usize) -> &'static DrumBank {
    &DRUM_BANKS[index % NUM_DRUM_BANKS]
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DrumState {
    /// All patterns: patterns[pattern_idx][voice][step]
    pub patterns: [[[bool; NUM_STEPS]; NUM_DRUM_VOICES]; NUM_PATTERNS],
    /// Currently selected pattern for editing (0-7)
    pub current_pattern: usize,
    /// Volumes per voice
    #[serde(alias = "vol")]
    pub volumes: [f32; NUM_DRUM_VOICES],
    /// Whether sequencer is running
    pub running: bool,
    /// Beats per minute
    pub bpm: f32,
    /// Current step position within the playing pattern
    #[serde(default)]
    pub current_step: usize,
    /// Selected drum bank
    pub bank: usize,
    /// Manual triggers (from keyboard/MIDI)
    #[serde(default)]
    pub triggers: [bool; NUM_DRUM_VOICES],
    /// Chain of pattern indices to play in order
    #[serde(default)]
    pub chain: Vec<usize>,
    /// Current position in chain during playback
    #[serde(default)]
    pub chain_position: usize,
    /// Whether chain mode is active (vs single pattern loop)
    #[serde(default)]
    pub chain_mode: bool,
}

impl DrumState {
    /// Get reference to current pattern's steps
    pub fn steps(&self) -> &[[bool; NUM_STEPS]; NUM_DRUM_VOICES] {
        &self.patterns[self.current_pattern]
    }

    /// Get mutable reference to current pattern's steps
    pub fn steps_mut(&mut self) -> &mut [[bool; NUM_STEPS]; NUM_DRUM_VOICES] {
        &mut self.patterns[self.current_pattern]
    }

    /// Get the pattern that should currently be playing
    pub fn playing_pattern(&self) -> usize {
        if self.chain_mode && !self.chain.is_empty() {
            self.chain[self.chain_position % self.chain.len()]
        } else {
            self.current_pattern
        }
    }

    /// Get steps for the currently playing pattern
    pub fn playing_steps(&self) -> &[[bool; NUM_STEPS]; NUM_DRUM_VOICES] {
        &self.patterns[self.playing_pattern()]
    }

    pub fn set_step(&mut self, voice: DrumVoice, step: usize, enabled: bool) {
        if step < NUM_STEPS {
            self.patterns[self.current_pattern][voice.index()][step] = enabled;
        }
    }

    pub fn clear_voice(&mut self, voice: DrumVoice) {
        self.patterns[self.current_pattern][voice.index()] = [false; NUM_STEPS];
    }

    pub fn clear_pattern(&mut self) {
        self.patterns[self.current_pattern] = [[false; NUM_STEPS]; NUM_DRUM_VOICES];
    }

    /// Clear current pattern (alias for clear_pattern for backward compat)
    pub fn clear_all(&mut self) {
        self.clear_pattern();
    }

    pub fn clear_all_patterns(&mut self) {
        self.patterns = [[[false; NUM_STEPS]; NUM_DRUM_VOICES]; NUM_PATTERNS];
    }

    pub fn set_level(&mut self, voice: DrumVoice, level: f32) {
        self.volumes[voice.index()] = level.clamp(0.0, 1.0);
    }

    /// Advance to next pattern in chain, returns true if chain wrapped
    pub fn advance_chain(&mut self) -> bool {
        if self.chain.is_empty() {
            return false;
        }
        self.chain_position += 1;
        if self.chain_position >= self.chain.len() {
            self.chain_position = 0;
            true
        } else {
            false
        }
    }

    /// Add a pattern to the chain
    pub fn chain_push(&mut self, pattern: usize) {
        if self.chain.len() < MAX_CHAIN_LENGTH && pattern < NUM_PATTERNS {
            self.chain.push(pattern);
        }
    }

    /// Remove last pattern from chain
    pub fn chain_pop(&mut self) {
        self.chain.pop();
        if self.chain_position >= self.chain.len() && !self.chain.is_empty() {
            self.chain_position = self.chain.len() - 1;
        }
    }

    /// Clear the chain
    pub fn chain_clear(&mut self) {
        self.chain.clear();
        self.chain_position = 0;
    }

    /// Copy current pattern to another slot
    pub fn copy_pattern_to(&mut self, dest: usize) {
        if dest < NUM_PATTERNS && dest != self.current_pattern {
            self.patterns[dest] = self.patterns[self.current_pattern];
        }
    }
}

impl Default for DrumState {
    fn default() -> Self {
        Self {
            patterns: [[[false; NUM_STEPS]; NUM_DRUM_VOICES]; NUM_PATTERNS],
            current_pattern: 0,
            volumes: [1.0; NUM_DRUM_VOICES],
            running: false,
            bpm: 120.0,
            current_step: 0,
            bank: 0,
            triggers: [false; NUM_DRUM_VOICES],
            chain: Vec::new(),
            chain_position: 0,
            chain_mode: false,
        }
    }
}
