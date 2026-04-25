use std::{
    fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result};

use crate::state::{project::ProjectTarget, AppState};

pub fn projects_dir(base_dir: &Path) -> PathBuf {
    base_dir.join("projects")
}

pub fn wav_dir(base_dir: &Path) -> PathBuf {
    base_dir.join("wav")
}

pub fn ensure_runtime_dirs(base_dir: &Path) -> Result<()> {
    fs::create_dir_all(projects_dir(base_dir)).context("create projects dir")?;
    fs::create_dir_all(wav_dir(base_dir)).context("create wav dir")?;
    Ok(())
}

pub fn list_projects(base_dir: &Path) -> Result<Vec<ProjectTarget>> {
    let mut items = Vec::new();
    let dir = projects_dir(base_dir);
    if !dir.exists() {
        return Ok(items);
    }

    for entry in fs::read_dir(dir).context("read projects dir")? {
        let entry = entry.context("read project entry")?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("mush") {
            if let Some(name) = path.file_name().and_then(|v| v.to_str()) {
                items.push(ProjectTarget {
                    name: name.to_string(),
                });
            }
        }
    }
    items.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(items)
}

pub fn save_project(base_dir: &Path, name: &str, state: &AppState) -> Result<PathBuf> {
    ensure_runtime_dirs(base_dir)?;
    let file_name = normalize_project_name(name);
    let path = projects_dir(base_dir).join(&file_name);
    let json = serde_json::to_string_pretty(state).context("serialize project")?;
    fs::write(&path, json).with_context(|| format!("write project {}", path.display()))?;
    Ok(path)
}

pub fn load_project(base_dir: &Path, name: &str) -> Result<AppState> {
    let file_name = normalize_project_name(name);
    let path = projects_dir(base_dir).join(&file_name);
    let text =
        fs::read_to_string(&path).with_context(|| format!("read project {}", path.display()))?;
    let state = serde_json::from_str::<AppState>(&text).context("parse project json")?;
    Ok(state)
}

pub fn next_wav_path(base_dir: &Path) -> Result<PathBuf> {
    ensure_runtime_dirs(base_dir)?;
    let base = wav_dir(base_dir).join("mush.wav");
    if !base.exists() {
        return Ok(base);
    }

    for idx in 1..10000 {
        let candidate = wav_dir(base_dir).join(format!("mush-{idx:04}.wav"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }

    anyhow::bail!("no free wav filename slot")
}

pub fn normalize_project_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.ends_with(".mush") {
        trimmed.to_string()
    } else {
        format!("{trimmed}.mush")
    }
}

/// Get the path for a project's associated loop WAV file.
pub fn loop_wav_path(base_dir: &Path, project_name: &str) -> PathBuf {
    let file_name = normalize_project_name(project_name);
    let loop_name = file_name.replace(".mush", ".loop.wav");
    projects_dir(base_dir).join(loop_name)
}

/// Mono sample captured from line-in (sidecar for `.mush` project).
pub fn sample_wav_path(base_dir: &Path, project_name: &str) -> PathBuf {
    let file_name = normalize_project_name(project_name);
    let name = file_name.replace(".mush", ".sample.wav");
    projects_dir(base_dir).join(name)
}

/// Mono buffer for sample-tab performance loop (chromatic take layer).
pub fn sample_loop_wav_path(base_dir: &Path, project_name: &str) -> PathBuf {
    let file_name = normalize_project_name(project_name);
    let name = file_name.replace(".mush", ".sampleloop.wav");
    projects_dir(base_dir).join(name)
}

pub fn save_sample_wav(path: &Path, samples: &[f32], sample_rate: u32) -> Result<()> {
    save_loop_wav(path, samples, sample_rate)
}

pub fn load_sample_wav(path: &Path) -> Result<(Vec<f32>, usize, u32)> {
    let mut reader = hound::WavReader::open(path).context("open sample wav")?;
    let spec = reader.spec();
    let rate = spec.sample_rate;
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().filter_map(|s| s.ok()).collect(),
        hound::SampleFormat::Int => match spec.bits_per_sample {
            16 => reader
                .samples::<i16>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / i16::MAX as f32)
                .collect(),
            24 | 32 => reader
                .samples::<i32>()
                .filter_map(|s| s.ok())
                .map(|s| s as f32 / i32::MAX as f32)
                .collect(),
            _ => anyhow::bail!("unsupported bit depth: {}", spec.bits_per_sample),
        },
    };
    let len = samples.len();
    Ok((samples, len, rate))
}

/// Save loop audio samples to WAV file.
pub fn save_loop_wav(path: &Path, samples: &[f32], sample_rate: u32) -> Result<()> {
    let spec = hound::WavSpec {
        channels: 1, // Loop is mono
        sample_rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut writer = hound::WavWriter::create(path, spec).context("create loop wav")?;
    for &sample in samples {
        writer.write_sample(sample)?;
    }
    writer.finalize()?;
    Ok(())
}

/// Load loop audio samples from WAV file.
/// Returns (samples, length) if successful.
pub fn load_loop_wav(path: &Path) -> Result<(Vec<f32>, usize)> {
    let mut reader = hound::WavReader::open(path).context("open loop wav")?;
    let spec = reader.spec();
    
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => {
            reader.samples::<f32>()
                .filter_map(|s| s.ok())
                .collect()
        }
        hound::SampleFormat::Int => {
            match spec.bits_per_sample {
                16 => {
                    reader.samples::<i16>()
                        .filter_map(|s| s.ok())
                        .map(|s| s as f32 / i16::MAX as f32)
                        .collect()
                }
                24 | 32 => {
                    reader.samples::<i32>()
                        .filter_map(|s| s.ok())
                        .map(|s| s as f32 / i32::MAX as f32)
                        .collect()
                }
                _ => anyhow::bail!("unsupported bit depth: {}", spec.bits_per_sample),
            }
        }
    };
    
    let len = samples.len();
    Ok((samples, len))
}
