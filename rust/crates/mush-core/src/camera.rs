use std::{
    io::Read,
    process::{Child, Command, Stdio},
    sync::Arc,
    thread,
};

use parking_lot::Mutex;

use crate::state::{audio::ReactiveLevels, ui::VisualFx};

pub const CAMERA_FRAME_WIDTH: usize = 160;
pub const CAMERA_FRAME_HEIGHT: usize = 90;

#[derive(Default)]
pub struct CameraShared {
    pub gray: Option<Vec<u8>>,
    pub error: String,
    pub running: bool,
}

pub struct CameraRuntime {
    shared: Arc<Mutex<CameraShared>>,
    _thread: thread::JoinHandle<()>,
}

impl CameraRuntime {
    pub fn start() -> Self {
        let shared = Arc::new(Mutex::new(CameraShared {
            gray: None,
            error: "Starting camera...".to_string(),
            running: true,
        }));
        let thread_shared = Arc::clone(&shared);
        let handle = thread::spawn(move || camera_worker(thread_shared));
        Self {
            shared,
            _thread: handle,
        }
    }

    pub fn render_ascii(
        &self,
        width: usize,
        height: usize,
        style: VisualFx,
        depth: f32,
        reactive: &ReactiveLevels,
    ) -> Vec<String> {
        let shared = self.shared.lock();
        let Some(gray) = &shared.gray else {
            let mut lines = vec![" ".repeat(width.max(1)); height.max(1)];
            if !lines.is_empty() {
                let msg = trim_to_width(
                    if shared.error.is_empty() {
                        "Camera idle"
                    } else {
                        &shared.error
                    },
                    width.max(1),
                );
                lines[height.saturating_div(2)] = center_text(&msg, width.max(1));
            }
            return lines;
        };

        let ramp: Vec<char> = match style {
            VisualFx::EdgePulse => " .'^:!*ox%#@".chars().collect(),
            VisualFx::FireStorm => " .,:;irsXA253hMHGS#9B&@".chars().collect(),
            VisualFx::IcePulse => "  .-~=+*#%@".chars().collect(),
            _ => " .:-=+*#%@".chars().collect(),
        };
        let mut lines = Vec::with_capacity(height);
        for y in 0..height.max(1) {
            let src_y = y * CAMERA_FRAME_HEIGHT / height.max(1);
            let mut line = String::with_capacity(width);
            for x in 0..width.max(1) {
                let mut src_x = x * CAMERA_FRAME_WIDTH / width.max(1);
                if matches!(
                    style,
                    VisualFx::GlitchShift | VisualFx::ChromaSplit
                ) {
                    let shift = ((reactive.kick + reactive.drums) * depth * 8.0) as usize;
                    src_x = (src_x
                        + if y % 2 == 0 {
                            shift
                        } else {
                            CAMERA_FRAME_WIDTH - (shift % CAMERA_FRAME_WIDTH.max(1))
                        })
                        % CAMERA_FRAME_WIDTH;
                }
                let idx = src_y * CAMERA_FRAME_WIDTH + src_x;
                let mut value = gray.get(idx).copied().unwrap_or(0) as f32 / 255.0;
                value = apply_visual_fx(
                    value,
                    x,
                    y,
                    width.max(1),
                    height.max(1),
                    style,
                    depth,
                    reactive,
                );
                let ramp_idx = ((value.clamp(0.0, 1.0)) * (ramp.len().saturating_sub(1)) as f32)
                    .round() as usize;
                line.push(ramp[ramp_idx.min(ramp.len().saturating_sub(1))]);
            }
            lines.push(line);
        }
        lines
    }
}

fn camera_worker(shared: Arc<Mutex<CameraShared>>) {
    let Some(mut child) = spawn_ffmpeg() else {
        shared.lock().error = "ffmpeg not found".to_string();
        return;
    };

    let frame_size = CAMERA_FRAME_WIDTH * CAMERA_FRAME_HEIGHT;
    let mut stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            shared.lock().error = "camera pipe unavailable".to_string();
            return;
        }
    };
    let mut buf = vec![0u8; frame_size];
    loop {
        match stdout.read_exact(&mut buf) {
            Ok(()) => {
                let mut state = shared.lock();
                state.gray = Some(buf.clone());
                state.error.clear();
            }
            Err(err) => {
                let mut state = shared.lock();
                state.error = format!("camera read failed: {err}");
                state.gray = None;
                break;
            }
        }
    }
    let _ = child.kill();
}

fn spawn_ffmpeg() -> Option<Child> {
    let ffmpeg = which_ffmpeg()?;
    let camera_device = std::env::var("MUSH_CAMERA_DEVICE").unwrap_or_else(|_| "0".to_string());
    Command::new(ffmpeg)
        .args([
            "-loglevel",
            "quiet",
            "-f",
            "avfoundation",
            "-framerate",
            "30",
            "-video_size",
            "640x480",
            "-i",
            &camera_device,
            "-vf",
            "fps=12,format=gray,eq=contrast=2.5:brightness=0.05,scale=160:90",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "gray",
            "-",
        ])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .spawn()
        .ok()
}

fn which_ffmpeg() -> Option<String> {
    std::env::var_os("PATH").and_then(|paths| {
        for path in std::env::split_paths(&paths) {
            let candidate = path.join("ffmpeg");
            if candidate.exists() {
                return Some(candidate.display().to_string());
            }
        }
        None
    })
}

/// Apply visual effects post-processing to a pixel value based on audio levels.
/// This is used by both camera and other visual effects for audio reactivity.
pub fn apply_visual_fx(
    value: f32,
    x: usize,
    y: usize,
    width: usize,
    height: usize,
    style: VisualFx,
    depth: f32,
    reactive: &ReactiveLevels,
) -> f32 {
    match style {
        VisualFx::Off => value,
        VisualFx::KickFlash => (value + reactive.kick * depth * 0.5).clamp(0.0, 1.0),
        VisualFx::SynthGlow => {
            let mid = width as f32 / 2.0;
            let dist = ((x as f32 - mid).abs() / mid.max(1.0)).clamp(0.0, 1.0);
            (value + (1.0 - dist) * reactive.synth * depth * 0.45).clamp(0.0, 1.0)
        }
        VisualFx::DrumPunch => {
            (1.0 - value * (0.7 - reactive.drums * depth * 0.3)).clamp(0.0, 1.0)
        }
        VisualFx::BassScan => {
            let scan =
                (((reactive.master * depth) * width as f32 * 0.5) as usize + y * 3) % width.max(1);
            if x.abs_diff(scan) < 3 {
                (value + 0.35).clamp(0.0, 1.0)
            } else {
                value
            }
        }
        VisualFx::EdgePulse => {
            (value * 0.3 + ((x + y) % 5) as f32 / 5.0 * reactive.kick * depth).clamp(0.0, 1.0)
        }
        VisualFx::GlitchShift => (value + reactive.drums * depth * 0.15).clamp(0.0, 1.0),
        VisualFx::GatePoster => {
            let levels = (3.0 + reactive.note * depth * 5.0).round().max(2.0);
            ((value * levels).floor() / levels).clamp(0.0, 1.0)
        }
        VisualFx::FireStorm => {
            (value + reactive.kick * depth * 0.25 + reactive.drums * depth * 0.18).clamp(0.0, 1.0)
        }
        VisualFx::IcePulse => (value * (0.85 + reactive.synth * depth * 0.25)
            + reactive.hat * depth * 0.12)
            .clamp(0.0, 1.0),
        VisualFx::ChromaSplit => {
            let band = ((reactive.note + reactive.synth) * depth * height as f32 * 0.1) as usize;
            if y.abs_diff(height / 2) < band.max(1) {
                (value + 0.2).clamp(0.0, 1.0)
            } else {
                value
            }
        }
        VisualFx::MatrixBeat => {
            if (x + y + ((reactive.kick * depth * 10.0) as usize)) % 7 == 0 {
                1.0
            } else {
                value * 0.6
            }
        }
        // New effects
        VisualFx::WaveRipple => {
            let cx = width as f32 / 2.0;
            let cy = height as f32 / 2.0;
            let dist = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            let wave = ((dist + reactive.synth * depth * 20.0) * 0.3).sin() * 0.5 + 0.5;
            (value * 0.5 + wave * reactive.synth * depth * 0.5).clamp(0.0, 1.0)
        }
        VisualFx::BeatStrobe => {
            let strobe = if reactive.master > 0.3 { reactive.master * depth } else { 0.0 };
            (value * (0.5 + strobe * 0.8)).clamp(0.0, 1.0)
        }
        VisualFx::HatSparkle => {
            // Deterministic "random" based on position, modulated by hat
            let pseudo_rand = ((x * 17 + y * 31 + (reactive.hat * 100.0) as usize) % 100) as f32 / 100.0;
            if reactive.hat > 0.4 && pseudo_rand > 0.85 {
                (value + reactive.hat * depth * 0.6).clamp(0.0, 1.0)
            } else {
                value
            }
        }
        VisualFx::SnareBurst => {
            let cx = width as f32 / 2.0;
            let cy = height as f32 / 2.0;
            let dist = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            let ring_radius = reactive.snare * depth * (width.min(height) as f32 * 0.4);
            if dist.abs_diff(ring_radius) < 3.0 {
                (value + reactive.snare * depth * 0.5).clamp(0.0, 1.0)
            } else {
                value * (1.0 - reactive.snare * 0.2)
            }
        }
        VisualFx::BassWobble => {
            // Horizontal wave distortion based on kick - creates wavy brightness pattern
            let wave_x = (x as f32 * 0.1).sin();
            let wave_y = (y as f32 * 0.3 + reactive.kick * depth * 10.0).sin();
            let wobble = wave_x * wave_y * reactive.kick * depth;
            (value * (0.7 + wobble * 0.5) + reactive.kick * depth * 0.2).clamp(0.0, 1.0)
        }
        VisualFx::LfoSweep => {
            // Scanning brightness bar based on LFO phase
            let lfo_pos = (reactive.lfo_phase * width as f32) as usize % width.max(1);
            let dist = x.abs_diff(lfo_pos);
            if dist < 5 {
                (value + (1.0 - dist as f32 / 5.0) * depth * 0.4).clamp(0.0, 1.0)
            } else {
                value * 0.7
            }
        }
        VisualFx::EnvelopeFade => {
            // Overall brightness controlled by envelope
            (value * (0.3 + reactive.env * depth * 0.9)).clamp(0.0, 1.0)
        }
        VisualFx::DrumGrid => {
            // Divide screen into 6 regions for 6 drum voices
            let region_w = width / 3;
            let region_h = height / 2;
            let region_x = x / region_w.max(1);
            let region_y = y / region_h.max(1);
            let region_idx = region_y * 3 + region_x;
            let intensity = match region_idx {
                0 => reactive.kick,
                1 => reactive.snare,
                2 => reactive.hat,
                3 => reactive.drums * 0.8,
                4 => reactive.master * 0.6,
                _ => reactive.synth * 0.7,
            };
            (value + intensity * depth * 0.35).clamp(0.0, 1.0)
        }
        VisualFx::FreqShift => {
            // Contrast shift based on frequency (higher = more contrast)
            let freq_factor = (reactive.freq / 1000.0).clamp(0.1, 2.0);
            let contrast = 0.5 + freq_factor * depth * 0.5;
            ((value - 0.5) * contrast + 0.5).clamp(0.0, 1.0)
        }
        VisualFx::ComboReact => {
            // Complex combination: kick=flash, snare=invert, hat=sparkle, synth=glow
            let mut v = value;
            // Kick flash
            v += reactive.kick * depth * 0.3;
            // Snare partial invert
            if reactive.snare > 0.4 {
                v = 1.0 - v * (1.0 - reactive.snare * 0.5);
            }
            // Hat sparkle
            let pseudo = ((x * 13 + y * 37) % 50) as f32 / 50.0;
            if reactive.hat > 0.3 && pseudo > 0.8 {
                v += 0.3;
            }
            // Synth center glow
            let mid = width as f32 / 2.0;
            let dist = ((x as f32 - mid).abs() / mid.max(1.0)).clamp(0.0, 1.0);
            v += (1.0 - dist) * reactive.synth * depth * 0.2;
            v.clamp(0.0, 1.0)
        }
    }
}

trait AbsDiff {
    fn abs_diff(self, other: Self) -> Self;
}
impl AbsDiff for f32 {
    fn abs_diff(self, other: Self) -> Self {
        (self - other).abs()
    }
}

fn center_text(text: &str, width: usize) -> String {
    let trimmed = trim_to_width(text, width);
    let pad = width.saturating_sub(trimmed.len()) / 2;
    format!(
        "{}{}{}",
        " ".repeat(pad),
        trimmed,
        " ".repeat(width.saturating_sub(pad + trimmed.len()))
    )
}

fn trim_to_width(text: &str, width: usize) -> String {
    text.chars().take(width).collect()
}
