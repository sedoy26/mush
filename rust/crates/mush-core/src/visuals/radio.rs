//! Radio Waves effect - polar sine wave concentric rings.
//!
//! Algorithm:
//! v(x,y,t) = sin(k·r − ω·t + φ(θ)) where r is radius from center
//! Rings appear/contract based on v. Phase modulation by angle for asymmetry.
//!
//! Tunables: wavelength, speed, center motion, rotation, thickness, palette

use crate::state::audio::ReactiveLevels;
use crate::visuals::util::{palette, ramp, TAU};
use crate::visuals::{Color, Framebuffer, ParamError, ParamKind, ParamSpec, ParamValue, Visual};

use rand::rngs::SmallRng;
use rand::SeedableRng;

pub struct Radio {
    time: f32,
    wavelength: f32,
    speed: f32,
    thickness: f32,
    asymmetry: f32,
    palette_idx: usize,
    palette: [Color; 256],
    #[allow(dead_code)]
    rng: SmallRng,
}

impl Radio {
    pub fn new(seed: u64) -> Self {
        Self {
            time: 0.0,
            wavelength: 5.0,
            speed: 3.0,
            thickness: 0.4,
            asymmetry: 0.0,
            palette_idx: 9, // neon
            palette: palette::neon(),
            rng: SmallRng::seed_from_u64(seed),
        }
    }

    fn params_list() -> &'static [ParamSpec] {
        static PARAMS: &[ParamSpec] = &[
            ParamSpec {
                key: "wavelength",
                label: "Wave",
                kind: ParamKind::Float { min: 2.0, max: 15.0, step: 0.5 },
                default: ParamValue::Float(5.0),
            },
            ParamSpec {
                key: "speed",
                label: "Speed",
                kind: ParamKind::Float { min: 1.0, max: 10.0, step: 0.5 },
                default: ParamValue::Float(3.0),
            },
            ParamSpec {
                key: "thickness",
                label: "Thick",
                kind: ParamKind::Float { min: 0.1, max: 1.0, step: 0.1 },
                default: ParamValue::Float(0.4),
            },
            ParamSpec {
                key: "asymmetry",
                label: "Asym",
                kind: ParamKind::Float { min: 0.0, max: 1.0, step: 0.1 },
                default: ParamValue::Float(0.0),
            },
            ParamSpec {
                key: "palette",
                label: "Palette",
                kind: ParamKind::Enum { variants: palette::PALETTE_NAMES },
                default: ParamValue::Enum(9),
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

impl Visual for Radio {
    fn name(&self) -> &'static str {
        "Radio"
    }

    fn params(&self) -> &[ParamSpec] {
        Self::params_list()
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<(), ParamError> {
        match key {
            "wavelength" => {
                if let ParamValue::Float(v) = value {
                    self.wavelength = v.clamp(2.0, 15.0);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            "speed" => {
                if let ParamValue::Float(v) = value {
                    self.speed = v.clamp(1.0, 10.0);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            "thickness" => {
                if let ParamValue::Float(v) = value {
                    self.thickness = v.clamp(0.1, 1.0);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            "asymmetry" => {
                if let ParamValue::Float(v) = value {
                    self.asymmetry = v.clamp(0.0, 1.0);
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
                        Err(ParamError::new("invalid palette"))
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
            "wavelength" => Some(ParamValue::Float(self.wavelength)),
            "speed" => Some(ParamValue::Float(self.speed)),
            "thickness" => Some(ParamValue::Float(self.thickness)),
            "asymmetry" => Some(ParamValue::Float(self.asymmetry)),
            "palette" => Some(ParamValue::Enum(self.palette_idx)),
            _ => None,
        }
    }

    fn tick(&mut self, dt: f32, reactive: &ReactiveLevels) {
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
        
        // Moving center (Lissajous)
        let center_x = cx + (self.time * 0.3).sin() * 5.0;
        let center_y = cy + (self.time * 0.5).cos() * 3.0;

        let k = TAU / self.wavelength;

        for yi in 0..h {
            for xi in 0..w {
                // Aspect ratio correction
                let dx = (xi as f32 - center_x) / 2.0;
                let dy = yi as f32 - center_y;
                
                let r = (dx * dx + dy * dy).sqrt();
                let theta = dy.atan2(dx);
                
                // Radio wave function with optional asymmetric phase
                let phase = self.asymmetry * (theta * 2.0).sin();
                let v = (k * r - self.time * TAU + phase).sin();
                
                // Convert to ring visibility
                let ring = (v + 1.0) * 0.5; // [0, 1]
                let in_ring = ring > (1.0 - self.thickness);
                
                if in_ring {
                    let intensity = (ring - (1.0 - self.thickness)) / self.thickness;
                    let color_idx = ((theta / TAU * 0.5 + 0.5 + self.time * 0.1) * 256.0) as usize % 256;
                    let color = self.palette[color_idx];
                    let ch = ramp::ramp(intensity, ramp::RAMP_BLOCKS);
                    fb.set_cell(xi, yi, ch, color, Color::BLACK);
                } else {
                    fb.set_cell(xi, yi, ' ', Color::BLACK, Color::BLACK);
                }
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
        let mut effect = Radio::new(12345);
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
        assert!(unique_chars.len() >= 2, "Expected some rings");
    }
}
