//! Kaleidoscope effect - polar-symmetric pattern with folding.
//!
//! Algorithm:
//! 1. Convert (x, y) to polar (r, θ) around center
//! 2. Fold: θ' = |((θ mod (2π/n)) − π/n)| where n = arm count
//! 3. Sample a time-varying pattern (plasma or noise) at (r, θ')
//! 4. Apply rotation animation
//!
//! Tunables: arm count (3-16), rotation speed, inner pattern, palette

use rand::rngs::SmallRng;
use rand::SeedableRng;

use crate::state::audio::ReactiveLevels;
use crate::visuals::util::{palette, ramp, math, TAU};
use crate::visuals::{Color, Framebuffer, ParamError, ParamKind, ParamSpec, ParamValue, Visual};

pub struct Kaleidoscope {
    time: f32,
    arms: i32,
    rotation_speed: f32,
    pattern_scale: f32,
    palette_idx: usize,
    palette: [Color; 256],
    #[allow(dead_code)]
    rng: SmallRng,
}

impl Kaleidoscope {
    pub fn new(seed: u64) -> Self {
        Self {
            time: 0.0,
            arms: 6,
            rotation_speed: 0.5,
            pattern_scale: 1.0,
            palette_idx: 5, // rainbow
            palette: palette::rainbow(),
            rng: SmallRng::seed_from_u64(seed),
        }
    }

    fn params_list() -> &'static [ParamSpec] {
        static PARAMS: &[ParamSpec] = &[
            ParamSpec {
                key: "arms",
                label: "Arms",
                kind: ParamKind::Int { min: 3, max: 16 },
                default: ParamValue::Int(6),
            },
            ParamSpec {
                key: "rotation",
                label: "Rotation",
                kind: ParamKind::Float { min: 0.0, max: 2.0, step: 0.1 },
                default: ParamValue::Float(0.5),
            },
            ParamSpec {
                key: "scale",
                label: "Scale",
                kind: ParamKind::Float { min: 0.2, max: 3.0, step: 0.1 },
                default: ParamValue::Float(1.0),
            },
            ParamSpec {
                key: "palette",
                label: "Palette",
                kind: ParamKind::Enum { variants: palette::PALETTE_NAMES },
                default: ParamValue::Enum(2), // rainbow
            },
        ];
        PARAMS
    }

    fn update_palette(&mut self) {
        if let Some(p) = palette::by_name(palette::PALETTE_NAMES[self.palette_idx]) {
            self.palette = p;
        }
    }
}

impl Visual for Kaleidoscope {
    fn name(&self) -> &'static str {
        "Kaleidoscope"
    }

    fn params(&self) -> &[ParamSpec] {
        Self::params_list()
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<(), ParamError> {
        match key {
            "arms" => {
                if let ParamValue::Int(v) = value {
                    self.arms = v.clamp(3, 16);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "int"))
                }
            }
            "rotation" => {
                if let ParamValue::Float(v) = value {
                    self.rotation_speed = v.clamp(0.0, 2.0);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            "scale" => {
                if let ParamValue::Float(v) = value {
                    self.pattern_scale = v.clamp(0.2, 3.0);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            "palette" => {
                if let ParamValue::Enum(idx) = value {
                    if idx < palette::PALETTE_NAMES.len() {
                        self.palette_idx = idx;
                        self.update_palette();
                        Ok(())
                    } else {
                        Err(ParamError::new("invalid palette index"))
                    }
                } else {
                    Err(ParamError::type_mismatch(key, "enum"))
                }
            }
            _ => Err(ParamError::unknown_key(key)),
        }
    }

    fn get_param(&self, key: &str) -> Option<ParamValue> {
        match key {
            "arms" => Some(ParamValue::Int(self.arms)),
            "rotation" => Some(ParamValue::Float(self.rotation_speed)),
            "scale" => Some(ParamValue::Float(self.pattern_scale)),
            "palette" => Some(ParamValue::Enum(self.palette_idx)),
            _ => None,
        }
    }

    fn tick(&mut self, dt: f32, reactive: &ReactiveLevels) {
        let speed_mod = 1.0 + reactive.master * 0.5 + reactive.kick * 0.3;
        self.time += dt * self.rotation_speed * speed_mod;
    }

    fn render(&self, fb: &mut Framebuffer) {
        let (w, h) = fb.size();
        if w == 0 || h == 0 {
            return;
        }

        let cx = w as f32 / 2.0;
        let cy = h as f32 / 2.0;
        let segment_angle = TAU / self.arms as f32;

        for yi in 0..h {
            for xi in 0..w {
                // Correct for aspect ratio
                let dx = (xi as f32 - cx) / 2.0;
                let dy = yi as f32 - cy;

                // Convert to polar
                let (r, mut theta) = math::to_polar(dx, dy);
                if theta < 0.0 {
                    theta += TAU;
                }

                // Apply rotation for pattern animation
                let rotated_theta = (theta + self.time) % TAU;

                // Fold into first segment for pattern symmetry
                let segment = (rotated_theta / segment_angle) as usize;
                let mut local_theta = rotated_theta % segment_angle;

                // Mirror alternate segments
                if segment % 2 == 1 {
                    local_theta = segment_angle - local_theta;
                }

                // Create pattern based on folded coordinates
                let spiral = (r * 0.15 * self.pattern_scale - local_theta * 3.0 + self.time * 2.0) % TAU;
                let rings = ((r * 0.3 * self.pattern_scale + self.time * 1.5) * 8.0).sin();
                let rays = (local_theta * 12.0 + self.time).cos();

                let pattern = spiral.sin() * 0.4 + rings * 0.3 + rays * 0.3;
                let v = ((pattern + 1.0) * 0.5).clamp(0.0, 1.0);

                // Use palette index based on pattern + position for varied colors
                // Combine angle, radius, and pattern for color variation
                let color_phase = (local_theta / segment_angle + r * 0.02 + self.time * 0.3 + pattern * 0.5) % 1.0;
                let palette_idx = ((color_phase * 255.0).abs() as usize) % 256;
                let base_color = self.palette[palette_idx];
                
                // Modulate brightness by pattern value
                let brightness = 0.3 + v * 0.7;
                let color = Color::rgb(
                    (base_color.r as f32 * brightness).min(255.0) as u8,
                    (base_color.g as f32 * brightness).min(255.0) as u8,
                    (base_color.b as f32 * brightness).min(255.0) as u8,
                );

                let ch = ramp::ramp(v, ramp::RAMP_EXTENDED);
                fb.set_cell(xi, yi, ch, color, Color::BLACK);
            }
        }
    }

    fn reset(&mut self) {
        self.time = 0.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_test() {
        let mut effect = Kaleidoscope::new(12345);
        let mut fb = Framebuffer::new(80, 24);
        let reactive = ReactiveLevels::default();

        for _ in 0..60 {
            effect.tick(1.0 / 60.0, &reactive);
        }
        effect.render(&mut fb);

        let mut unique_chars = std::collections::HashSet::new();
        for y in 0..24 {
            for x in 0..80 {
                if let Some(cell) = fb.get_cell(x, y) {
                    unique_chars.insert(cell.ch);
                }
            }
        }
        assert!(unique_chars.len() >= 5, "Expected varied output");
    }
}
