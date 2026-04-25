//! CPAL input stream for recording a mono sample from the selected device.

use std::sync::Arc;

use anyhow::{Context, Result};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{SampleFormat, SupportedStreamConfig};
use parking_lot::Mutex;

use crate::state::{audio::AudioDeviceSelection, sample::MAX_SAMPLE_SECONDS};

pub struct InputRecorder {
    pub recording: bool,
    pub buf: Vec<f32>,
    pub max_samples: usize,
    pub channels: u16,
    pub sample_rate: u32,
}

impl InputRecorder {
    fn new(max_samples: usize, channels: u16, sample_rate: u32) -> Self {
        Self {
            recording: false,
            buf: Vec::new(),
            max_samples,
            channels: channels.max(1),
            sample_rate,
        }
    }

    fn push_interleaved_f32(&mut self, data: &[f32]) {
        if !self.recording {
            return;
        }
        let ch = self.channels as usize;
        if ch == 0 {
            return;
        }
        let frames = data.len() / ch;
        for f in 0..frames {
            let base = f * ch;
            let mono: f32 = data[base..base + ch].iter().sum::<f32>() / ch as f32;
            if self.buf.len() < self.max_samples {
                self.buf.push(mono);
            }
        }
    }
}

fn select_input_device(
    host: &cpal::Host,
    selection: &AudioDeviceSelection,
) -> Result<cpal::Device> {
    match selection {
        AudioDeviceSelection::DefaultSystem => host
            .default_input_device()
            .context("no default input device"),
        AudioDeviceSelection::Named(name) => {
            for device in host.input_devices().context("enumerate input devices")? {
                if device_display_name(&device) == *name {
                    return Ok(device);
                }
            }
            host.default_input_device()
                .context("named input missing and no default input")
        }
    }
}

fn device_display_name(device: &cpal::Device) -> String {
    device
        .description()
        .map(|description| description.name().to_string())
        .unwrap_or_else(|_| "default".to_string())
}

/// Pick a stable format/rate from `supported_input_configs`, falling back to the device default.
fn best_supported_input_config(device: &cpal::Device) -> Result<SupportedStreamConfig> {
    let supported = match device.supported_input_configs() {
        Ok(s) => s,
        Err(_) => return device.default_input_config().context("input default config"),
    };
    let mut ranges: Vec<_> = supported.collect();
    if ranges.is_empty() {
        return device.default_input_config().context("input default config (no ranges)");
    }
    // Prefer F32, then I16, then U16; prefer fewer channels (mono) when tied.
    ranges.sort_by(|a, b| {
        let fmt_rank = |f: SampleFormat| match f {
            SampleFormat::F32 => 0u8,
            SampleFormat::I16 => 1,
            SampleFormat::U16 => 2,
            _ => 3,
        };
        fmt_rank(a.sample_format())
            .cmp(&fmt_rank(b.sample_format()))
            .then_with(|| a.channels().cmp(&b.channels()))
    });
    let preferred_rates = [48_000u32, 44_100, 96_000, 88_200, 32_000];
    for r in &ranges {
        for &pr in &preferred_rates {
            if let Some(c) = r.try_with_sample_rate(pr) {
                return Ok(c);
            }
        }
    }
    let r = ranges[0];
    let mid = (r.min_sample_rate() + r.max_sample_rate()) / 2;
    let sr = mid.clamp(r.min_sample_rate(), r.max_sample_rate());
    Ok(r.with_sample_rate(sr))
}

pub struct InputCapture {
    _stream: cpal::Stream,
    shared: Arc<Mutex<InputRecorder>>,
}

impl InputCapture {
    /// Start an input stream; samples accumulate only while `set_recording(true)`.
    pub fn start(selection: &AudioDeviceSelection, sample_rate_hint: f32) -> Result<Self> {
        let host = cpal::default_host();
        let device = select_input_device(&host, selection)?;
        let supported = best_supported_input_config(&device)?;
        let sample_format = supported.sample_format();
        let stream_config = supported.config();
        let rate: f32 = {
            let r: u32 = stream_config.sample_rate.into();
            r as f32
        };
        let rate = if rate < 1000.0 { sample_rate_hint } else { rate };
        let max_samples = (MAX_SAMPLE_SECONDS * rate) as usize;
        let channels = stream_config.channels.max(1);

        let sr_u32 = rate as u32;
        let shared = Arc::new(Mutex::new(InputRecorder::new(
            max_samples,
            channels,
            sr_u32,
        )));
        let shared_cb = Arc::clone(&shared);

        let err_fn = |e: cpal::StreamError| {
            eprintln!("[mush] sample input stream error: {e}");
        };

        let stream = match sample_format {
            cpal::SampleFormat::F32 => device.build_input_stream(
                &stream_config,
                move |data: &[f32], _| {
                    let mut g = shared_cb.lock();
                    g.push_interleaved_f32(data);
                },
                err_fn,
                None,
            )?,
            cpal::SampleFormat::I16 => {
                let shared_cb = Arc::clone(&shared);
                device.build_input_stream(
                    &stream_config,
                    move |data: &[i16], _| {
                        let v: Vec<f32> = data
                            .iter()
                            .map(|s| *s as f32 / i16::MAX as f32)
                            .collect();
                        let mut g = shared_cb.lock();
                        g.push_interleaved_f32(&v);
                    },
                    err_fn,
                    None,
                )?
            }
            cpal::SampleFormat::U16 => {
                let shared_cb = Arc::clone(&shared);
                device.build_input_stream(
                    &stream_config,
                    move |data: &[u16], _| {
                        let v: Vec<f32> = data
                            .iter()
                            .map(|s| (*s as f32 / u16::MAX as f32) * 2.0 - 1.0)
                            .collect();
                        let mut g = shared_cb.lock();
                        g.push_interleaved_f32(&v);
                    },
                    err_fn,
                    None,
                )?
            }
            other => anyhow::bail!("unsupported input sample format: {other:?}"),
        };

        // Arm recording before `play()` so the first callback frames are not dropped.
        {
            let mut g = shared.lock();
            g.recording = true;
            g.buf.clear();
        }
        stream.play()?;
        Ok(Self {
            _stream: stream,
            shared,
        })
    }

    pub fn shared(&self) -> Arc<Mutex<InputRecorder>> {
        Arc::clone(&self.shared)
    }

    pub fn set_recording(&self, on: bool) {
        let mut g = self.shared.lock();
        g.recording = on;
        if on {
            g.buf.clear();
        }
    }

    /// Stop capture and consume `self` so the input stream is dropped.
    pub fn stop_and_take(self) -> (Vec<f32>, u32) {
        let mut g = self.shared.lock();
        g.recording = false;
        let buf = std::mem::take(&mut g.buf);
        let sr = g.sample_rate;
        drop(g);
        drop(self);
        (buf, sr)
    }
}
