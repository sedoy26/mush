//! Plasma effect - classic demoscene sine wave interference pattern.
//!
//! Algorithm:
//! For each pixel (x, y) at time t, compute:
//!   v(x,y,t) = sin(a·x + t) + sin(b·y + t) + sin(c·(x+y) + t) + sin(d·r + t)
//! where r = distance from center, and a,b,c,d are frequency coefficients.
//! Normalize to [0,1], map through palette and character ramp.
//!
//! Tunables: speed, scale, palette, mode (which sine terms are active)

use rand::rngs::SmallRng;
use rand::SeedableRng;

use crate::state::audio::ReactiveLevels;
use crate::visuals::util::{palette, ramp};
use crate::visuals::{Color, Framebuffer, ParamError, ParamKind, ParamSpec, ParamValue, Visual};

/// Plasma visual effect
pub struct Plasma {
    time: f32,
    speed: f32,
    scale: f32,
    palette_idx: usize,
    ramp_idx: usize,
    palette: [Color; 256],
    #[allow(dead_code)]
    rng: SmallRng,
}

impl Plasma {
    /// Create a new plasma effect with the given seed.
    pub fn new(seed: u64) -> Self {
        Self {
            time: 0.0,
            speed: 1.0,
            scale: 1.0,
            palette_idx: 0,
            ramp_idx: 0,
            palette: palette::plasma(),
            rng: SmallRng::seed_from_u64(seed),
        }
    }

    fn params_list() -> &'static [ParamSpec] {
        static PARAMS: &[ParamSpec] = &[
            ParamSpec {
                key: "speed",
                label: "Speed",
                kind: ParamKind::Float {
                    min: 0.1,
                    max: 5.0,
                    step: 0.1,
                },
                default: ParamValue::Float(1.0),
            },
            ParamSpec {
                key: "scale",
                label: "Scale",
                kind: ParamKind::Float {
                    min: 0.2,
                    max: 4.0,
                    step: 0.1,
                },
                default: ParamValue::Float(1.0),
            },
            ParamSpec {
                key: "palette",
                label: "Palette",
                kind: ParamKind::Enum {
                    variants: palette::PALETTE_NAMES,
                },
                default: ParamValue::Enum(4), // plasma
            },
            ParamSpec {
                key: "ramp",
                label: "Char Ramp",
                kind: ParamKind::Enum {
                    variants: ramp::RAMP_NAMES,
                },
                default: ParamValue::Enum(4), // extended
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

impl Visual for Plasma {
    fn name(&self) -> &'static str {
        "Plasma"
    }

    fn params(&self) -> &[ParamSpec] {
        Self::params_list()
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<(), ParamError> {
        match key {
            "speed" => {
                if let ParamValue::Float(v) = value {
                    self.speed = v.clamp(0.1, 5.0);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            "scale" => {
                if let ParamValue::Float(v) = value {
                    self.scale = v.clamp(0.2, 4.0);
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
            "ramp" => {
                if let ParamValue::Enum(idx) = value {
                    if idx < ramp::RAMP_NAMES.len() {
                        self.ramp_idx = idx;
                        Ok(())
                    } else {
                        Err(ParamError::new("invalid ramp index"))
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
            "speed" => Some(ParamValue::Float(self.speed)),
            "scale" => Some(ParamValue::Float(self.scale)),
            "palette" => Some(ParamValue::Enum(self.palette_idx)),
            "ramp" => Some(ParamValue::Enum(self.ramp_idx)),
            _ => None,
        }
    }

    fn tick(&mut self, dt: f32, reactive: &ReactiveLevels) {
        // Speed modulated by master audio level
        let speed_mod = 1.0 + reactive.master * 0.5;
        self.time += dt * self.speed * speed_mod;
    }

    fn render(&self, fb: &mut Framebuffer) {
        let (w, h) = fb.size();
        if w == 0 || h == 0 {
            return;
        }

        let cx = w as f32 / 2.0;
        let cy = h as f32 / 2.0;
        let char_ramp = ramp::by_name(ramp::RAMP_NAMES[self.ramp_idx]);

        // Frequency coefficients affected by scale
        let a = 0.08 * self.scale;
        let b = 0.06 * self.scale;
        let c = 0.05 * self.scale;
        let d = 0.03 * self.scale;

        for yi in 0..h {
            for xi in 0..w {
                // Correct for terminal character aspect ratio (~2:1)
                let x = (xi as f32 - cx) / 2.0;
                let y = yi as f32 - cy;

                // Distance from center
                let r = (x * x + y * y).sqrt();

                // Plasma function: sum of sines with phase offsets
                let v1 = (a * x + self.time).sin();
                let v2 = (b * y + self.time * 0.7).sin();
                let v3 = (c * (x + y) + self.time * 1.2).sin();
                let v4 = (d * r + self.time * 0.9).sin();

                // Combine and normalize to [0, 1]
                let v = (v1 + v2 + v3 + v4) / 4.0 * 0.5 + 0.5;
                let v = v.clamp(0.0, 1.0);

                // Time-shifting color index for animation
                let color_shift = (self.time * 30.0) as usize;
                let color_idx = ((v * 255.0) as usize + color_shift) % 256;
                let color = self.palette[color_idx];

                let ch = ramp::ramp(v, char_ramp);
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
        let mut effect = Plasma::new(12345);
        let mut fb = Framebuffer::new(80, 24);
        let reactive = ReactiveLevels::default();

        // Tick 60 frames
        for _ in 0..60 {
            effect.tick(1.0 / 60.0, &reactive);
        }
        effect.render(&mut fb);

        // Verify no panic and variety of output
        let mut unique_chars = std::collections::HashSet::new();
        for y in 0..24 {
            for x in 0..80 {
                if let Some(cell) = fb.get_cell(x, y) {
                    unique_chars.insert(cell.ch);
                }
            }
        }
        assert!(
            unique_chars.len() >= 5,
            "Expected varied output, got {} unique chars",
            unique_chars.len()
        );
    }

    #[test]
    fn test_params() {
        let mut effect = Plasma::new(42);
        assert!(effect.set_param("speed", ParamValue::Float(2.0)).is_ok());
        assert_eq!(effect.get_param("speed"), Some(ParamValue::Float(2.0)));
        assert!(effect.set_param("unknown", ParamValue::Float(1.0)).is_err());
    }

    #[test]
    fn deterministic() {
        let mut effect1 = Plasma::new(12345);
        let mut effect2 = Plasma::new(12345);
        let mut fb1 = Framebuffer::new(40, 20);
        let mut fb2 = Framebuffer::new(40, 20);
        let reactive = ReactiveLevels::default();

        // Same seed + same dt sequence should produce identical frames
        for _ in 0..30 {
            effect1.tick(1.0 / 60.0, &reactive);
            effect2.tick(1.0 / 60.0, &reactive);
        }

        effect1.render(&mut fb1);
        effect2.render(&mut fb2);

        for y in 0..20 {
            for x in 0..40 {
                let c1 = fb1.get_cell(x, y).map(|c| c.ch);
                let c2 = fb2.get_cell(x, y).map(|c| c.ch);
                assert_eq!(c1, c2, "Mismatch at ({}, {})", x, y);
            }
        }
    }
}
