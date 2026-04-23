//! Matrix Rain effect - falling character streams with trailing fade.
//!
//! Algorithm:
//! One drop per column: struct Drop { y, speed, length, glyphs }
//! Per frame: y += speed·dt; draw head bright, fade trail above
//! Respawn at y = -length with new random speed/length when head passes bottom
//! Occasionally re-roll individual glyphs in trail for "shimmer"
//!
//! Tunables: density, speed range, glyph set, fade curve, head brightness

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

use crate::state::audio::ReactiveLevels;
use crate::visuals::util::ramp;
use crate::visuals::{Color, Framebuffer, ParamError, ParamKind, ParamSpec, ParamValue, Visual};

const MAX_COLS: usize = 256;
const MAX_TRAIL: usize = 32;

struct Drop {
    y: f32,
    speed: f32,
    length: usize,
    glyphs: [char; MAX_TRAIL],
    shimmer_timer: f32,
}

impl Default for Drop {
    fn default() -> Self {
        Self {
            y: 0.0,
            speed: 15.0,
            length: 10,
            glyphs: [' '; MAX_TRAIL],
            shimmer_timer: 0.0,
        }
    }
}

pub struct MatrixRain {
    drops: [Drop; MAX_COLS],
    active_cols: [bool; MAX_COLS],
    density: f32,
    speed_min: f32,
    speed_max: f32,
    ramp_idx: usize,
    head_brightness: f32,
    rng: SmallRng,
    time: f32,
}

impl MatrixRain {
    pub fn new(seed: u64) -> Self {
        let mut rng = SmallRng::seed_from_u64(seed);
        let drops: [Drop; MAX_COLS] = std::array::from_fn(|_| Drop::default());
        let mut active_cols = [false; MAX_COLS];
        
        // Initialize some active columns
        for col in active_cols.iter_mut().take(MAX_COLS) {
            *col = rng.gen::<f32>() < 0.3;
        }

        Self {
            drops,
            active_cols,
            density: 0.3,
            speed_min: 10.0,
            speed_max: 30.0,
            ramp_idx: 7, // matrix
            head_brightness: 1.0,
            rng,
            time: 0.0,
        }
    }

    fn spawn_drop(&mut self, col: usize, _height: u16) {
        let length = self.rng.gen_range(6..=MAX_TRAIL.min(20));
        let speed = self.rng.gen_range(self.speed_min..=self.speed_max);
        
        let chars = ramp::by_name(ramp::RAMP_NAMES[self.ramp_idx]);
        let mut glyphs = [' '; MAX_TRAIL];
        for g in glyphs.iter_mut().take(length) {
            let idx = self.rng.gen_range(0..chars.len());
            *g = chars[idx];
        }

        self.drops[col] = Drop {
            y: -(length as f32),
            speed,
            length,
            glyphs,
            shimmer_timer: 0.0,
        };
        self.active_cols[col] = true;
    }

    fn shimmer_glyph(&mut self, col: usize) {
        let chars = ramp::by_name(ramp::RAMP_NAMES[self.ramp_idx]);
        if !chars.is_empty() && self.drops[col].length > 0 {
            let idx = self.rng.gen_range(0..self.drops[col].length);
            let char_idx = self.rng.gen_range(0..chars.len());
            self.drops[col].glyphs[idx] = chars[char_idx];
        }
    }

    fn params_list() -> &'static [ParamSpec] {
        static PARAMS: &[ParamSpec] = &[
            ParamSpec {
                key: "density",
                label: "Density",
                kind: ParamKind::Float { min: 0.1, max: 1.0, step: 0.05 },
                default: ParamValue::Float(0.3),
            },
            ParamSpec {
                key: "speed_min",
                label: "Min Speed",
                kind: ParamKind::Float { min: 5.0, max: 20.0, step: 1.0 },
                default: ParamValue::Float(10.0),
            },
            ParamSpec {
                key: "speed_max",
                label: "Max Speed",
                kind: ParamKind::Float { min: 15.0, max: 50.0, step: 1.0 },
                default: ParamValue::Float(30.0),
            },
            ParamSpec {
                key: "ramp",
                label: "Glyph Set",
                kind: ParamKind::Enum { variants: ramp::RAMP_NAMES },
                default: ParamValue::Enum(7), // matrix
            },
        ];
        PARAMS
    }
}

impl Visual for MatrixRain {
    fn name(&self) -> &'static str {
        "MatrixRain"
    }

    fn params(&self) -> &[ParamSpec] {
        Self::params_list()
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<(), ParamError> {
        match key {
            "density" => {
                if let ParamValue::Float(v) = value {
                    self.density = v.clamp(0.1, 1.0);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            "speed_min" => {
                if let ParamValue::Float(v) = value {
                    self.speed_min = v.clamp(5.0, 20.0);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            "speed_max" => {
                if let ParamValue::Float(v) = value {
                    self.speed_max = v.clamp(15.0, 50.0);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
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
            "density" => Some(ParamValue::Float(self.density)),
            "speed_min" => Some(ParamValue::Float(self.speed_min)),
            "speed_max" => Some(ParamValue::Float(self.speed_max)),
            "ramp" => Some(ParamValue::Enum(self.ramp_idx)),
            _ => None,
        }
    }

    fn tick(&mut self, dt: f32, reactive: &ReactiveLevels) {
        self.time += dt;
        let speed_mod = 1.0 + reactive.master * 0.3;

        // Spawn new drops occasionally based on density
        let spawn_chance = self.density * dt * 3.0;
        for col in 0..MAX_COLS {
            if !self.active_cols[col] && self.rng.gen::<f32>() < spawn_chance {
                self.spawn_drop(col, 40); // Height doesn't matter for spawn
            }
        }

        // Update existing drops
        for col in 0..MAX_COLS {
            if self.active_cols[col] {
                self.drops[col].y += self.drops[col].speed * dt * speed_mod;
                
                // Deactivate drops that have fallen far off-screen (so they can respawn)
                // Use generous max since we don't know terminal height here
                let max_y = 150.0 + self.drops[col].length as f32;
                if self.drops[col].y > max_y {
                    self.active_cols[col] = false;
                }
                
                // Shimmer effect
                self.drops[col].shimmer_timer += dt;
                if self.drops[col].shimmer_timer > 0.1 && self.rng.gen::<f32>() < 0.3 {
                    self.shimmer_glyph(col);
                    self.drops[col].shimmer_timer = 0.0;
                }
            }
        }
    }

    fn render(&self, fb: &mut Framebuffer) {
        let (w, h) = fb.size();
        if w == 0 || h == 0 {
            return;
        }

        fb.clear();

        for col in 0..(w as usize).min(MAX_COLS) {
            if !self.active_cols[col] {
                continue;
            }

            let drop = &self.drops[col];
            let head_y = drop.y as i32;

            // Check if drop has passed beyond screen
            if head_y > (h as i32 + drop.length as i32) {
                continue;
            }

            for i in 0..drop.length {
                let trail_y = head_y - i as i32;
                if trail_y < 0 || trail_y >= h as i32 {
                    continue;
                }

                let glyph = drop.glyphs[i];
                
                // Brightness fades along trail
                let brightness = if i == 0 {
                    self.head_brightness
                } else {
                    1.0 - (i as f32 / drop.length as f32) * 0.9
                };

                let green = (brightness * 255.0) as u8;
                let white_mix = if i == 0 { 150 } else { 0 };
                let color = Color::rgb(white_mix, green, white_mix / 3);

                fb.set_cell(col as u16, trail_y as u16, glyph, color, Color::BLACK);
            }
        }
    }

    fn reset(&mut self) {
        self.time = 0.0;
        for col in self.active_cols.iter_mut() {
            *col = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_test() {
        let mut effect = MatrixRain::new(12345);
        let mut fb = Framebuffer::new(80, 24);
        let reactive = ReactiveLevels::default();

        for _ in 0..120 {
            effect.tick(1.0 / 60.0, &reactive);
        }
        effect.render(&mut fb);

        let mut unique_chars = std::collections::HashSet::new();
        for y in 0..24 {
            for x in 0..80 {
                if let Some(cell) = fb.get_cell(x, y) {
                    if cell.ch != ' ' {
                        unique_chars.insert(cell.ch);
                    }
                }
            }
        }
        // Matrix rain should produce variety after 2 seconds
        assert!(unique_chars.len() >= 3, "Expected varied output");
    }
}
