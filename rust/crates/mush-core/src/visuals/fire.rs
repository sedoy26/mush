//! Fire effect - cellular automaton flame simulation.
//!
//! Algorithm:
//! Heat buffer sized W × H. Bottom row: randomize to high heat with occasional gaps.
//! Propagation bottom-to-top:
//!   heat[y-1][x] = avg(heat[y][x-1], heat[y][x], heat[y][x+1], heat[y+1][x]) / 4 - cooling
//! Map heat to (color, char) via fire palette.
//!
//! Tunables: cooling rate, bottom intensity, wind (horizontal bias), palette

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

use crate::state::audio::ReactiveLevels;
use crate::visuals::util::{palette, ramp};
use crate::visuals::{Color, Framebuffer, ParamError, ParamKind, ParamSpec, ParamValue, Visual};

const MAX_WIDTH: usize = 256;
const MAX_HEIGHT: usize = 64;

pub struct Fire {
    heat: [[u8; MAX_WIDTH]; MAX_HEIGHT],
    cooling: f32,
    intensity: f32,
    wind: f32,
    palette_idx: usize,
    palette: [Color; 256],
    rng: SmallRng,
    width: usize,
    height: usize,
}

impl Fire {
    pub fn new(seed: u64) -> Self {
        Self {
            heat: [[0; MAX_WIDTH]; MAX_HEIGHT],
            cooling: 3.0,
            intensity: 0.9,
            wind: 0.0,
            palette_idx: 0, // fire
            palette: palette::fire(),
            rng: SmallRng::seed_from_u64(seed),
            width: 120,
            height: 40,
        }
    }

    fn params_list() -> &'static [ParamSpec] {
        static PARAMS: &[ParamSpec] = &[
            ParamSpec {
                key: "cooling",
                label: "Cooling",
                kind: ParamKind::Float { min: 0.5, max: 10.0, step: 0.5 },
                default: ParamValue::Float(3.0),
            },
            ParamSpec {
                key: "intensity",
                label: "Intensity",
                kind: ParamKind::Float { min: 0.3, max: 1.0, step: 0.05 },
                default: ParamValue::Float(0.9),
            },
            ParamSpec {
                key: "wind",
                label: "Wind",
                kind: ParamKind::Float { min: -1.0, max: 1.0, step: 0.1 },
                default: ParamValue::Float(0.0),
            },
            ParamSpec {
                key: "palette",
                label: "Palette",
                kind: ParamKind::Enum { variants: palette::PALETTE_NAMES },
                default: ParamValue::Enum(0), // fire
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

impl Visual for Fire {
    fn name(&self) -> &'static str {
        "Fire"
    }

    fn params(&self) -> &[ParamSpec] {
        Self::params_list()
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<(), ParamError> {
        match key {
            "cooling" => {
                if let ParamValue::Float(v) = value {
                    self.cooling = v.clamp(0.5, 10.0);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            "intensity" => {
                if let ParamValue::Float(v) = value {
                    self.intensity = v.clamp(0.3, 1.0);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            "wind" => {
                if let ParamValue::Float(v) = value {
                    self.wind = v.clamp(-1.0, 1.0);
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
            "cooling" => Some(ParamValue::Float(self.cooling)),
            "intensity" => Some(ParamValue::Float(self.intensity)),
            "wind" => Some(ParamValue::Float(self.wind)),
            "palette" => Some(ParamValue::Enum(self.palette_idx)),
            _ => None,
        }
    }

    fn tick(&mut self, _dt: f32, reactive: &ReactiveLevels) {
        let h = self.height.min(MAX_HEIGHT);
        let w = self.width.min(MAX_WIDTH);
        if h == 0 || w == 0 {
            return;
        }

        // Intensity boosted by audio
        let intensity = self.intensity + reactive.kick * 0.1 + reactive.master * 0.05;

        // Generate heat at bottom row
        let bottom = h - 1;
        for x in 0..w {
            if self.rng.gen::<f32>() < intensity {
                self.heat[bottom][x] = self.rng.gen_range(200..=255);
            } else {
                self.heat[bottom][x] = self.rng.gen_range(0..=100);
            }
        }

        // Propagate heat upward
        for y in (0..bottom).rev() {
            for x in 0..w {
                // Wind bias
                let offset = (self.wind * 2.0) as i32;
                let x_left = ((x as i32 - 1 + offset).max(0) as usize).min(w - 1);
                let x_right = ((x as i32 + 1 + offset).max(0) as usize).min(w - 1);
                
                let sum = self.heat[y + 1][x_left] as u32
                    + self.heat[y + 1][x] as u32
                    + self.heat[y + 1][x_right] as u32
                    + self.heat[y][x] as u32;
                
                let avg = (sum / 4) as i32;
                let cooling = self.rng.gen_range(0..=(self.cooling as i32 + 1));
                self.heat[y][x] = (avg - cooling).max(0) as u8;
            }
        }
    }

    fn render(&self, fb: &mut Framebuffer) {
        let (w, h) = fb.size();
        if w == 0 || h == 0 {
            return;
        }

        for yi in 0..h {
            for xi in 0..w {
                let heat = self.heat[yi as usize % MAX_HEIGHT][xi as usize % MAX_WIDTH];
                let color = self.palette[heat as usize];
                let intensity = heat as f32 / 255.0;
                let ch = ramp::ramp(intensity, ramp::RAMP_BLOCKS);
                fb.set_cell(xi, yi, ch, color, Color::BLACK);
            }
        }
    }

    fn reset(&mut self) {
        for row in self.heat.iter_mut() {
            for cell in row.iter_mut() {
                *cell = 0;
            }
        }
    }

    fn resize(&mut self, width: u16, height: u16) {
        self.width = (width as usize).min(MAX_WIDTH);
        self.height = (height as usize).min(MAX_HEIGHT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_test() {
        let mut effect = Fire::new(12345);
        effect.width = 80;
        effect.height = 24;
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
        assert!(unique_chars.len() >= 3, "Expected varied output");
    }
}
