//! Tunnel effect - flying through a textured cylindrical tunnel.
//!
//! Algorithm:
//! Precompute per-cell (depth, angle) LUTs:
//!   depth[y][x] = k / sqrt((x-cx)² + (y-cy)²)
//!   angle[y][x] = atan2(y-cy, x-cx) · N / π
//! Per frame: sample tex(depth + t, angle + t/2) where tex is a 2D pattern
//!
//! Tunables: shape (round, square, diamond), palette, speed, depth factor, texture

use crate::state::audio::ReactiveLevels;
use crate::visuals::util::{palette, ramp, math, TAU};
use crate::visuals::{Color, Framebuffer, ParamError, ParamKind, ParamSpec, ParamValue, Visual};

use rand::rngs::SmallRng;
use rand::SeedableRng;

const TEXTURE_VARIANTS: &[&str] = &["checker", "stripes", "plasma", "bricks"];
const SHAPE_VARIANTS: &[&str] = &["round", "square", "diamond"];

pub struct Tunnel {
    time: f32,
    speed: f32,
    depth_factor: f32,
    texture_idx: usize,
    shape_idx: usize,
    palette_idx: usize,
    palette: [Color; 256],
    #[allow(dead_code)]
    rng: SmallRng,
}

impl Tunnel {
    pub fn new(seed: u64) -> Self {
        Self {
            time: 0.0,
            speed: 2.0,
            depth_factor: 20.0,
            texture_idx: 0,
            shape_idx: 0,
            palette_idx: 2, // rainbow
            palette: palette::rainbow(),
            rng: SmallRng::seed_from_u64(seed),
        }
    }

    fn sample_texture(&self, u: f32, v: f32) -> f32 {
        match self.texture_idx {
            0 => {
                // Checker pattern
                let cu = (u * 4.0).floor() as i32;
                let cv = (v * 4.0).floor() as i32;
                if (cu + cv) % 2 == 0 { 1.0 } else { 0.0 }
            }
            1 => {
                // Stripes
                let stripe = (u * 8.0).sin() * 0.5 + 0.5;
                stripe
            }
            2 => {
                // Plasma
                let v1 = (u * 3.0).sin();
                let v2 = (v * 3.0).cos();
                let v3 = ((u + v) * 2.0).sin();
                ((v1 + v2 + v3) / 3.0 * 0.5 + 0.5).clamp(0.0, 1.0)
            }
            _ => {
                // Bricks
                let row = (v * 6.0).floor() as i32;
                let offset = if row % 2 == 0 { 0.5 } else { 0.0 };
                let brick_u = (u + offset).fract();
                let brick_v = (v * 6.0).fract();
                
                let mortar = 0.1;
                if brick_u < mortar || brick_v < mortar {
                    0.2 // mortar
                } else {
                    0.8 // brick
                }
            }
        }
    }

    fn distance_from_center(&self, dx: f32, dy: f32) -> f32 {
        match self.shape_idx {
            0 => (dx * dx + dy * dy).sqrt(), // Round (L2 norm)
            1 => dx.abs().max(dy.abs()),      // Square (L∞ norm)
            _ => dx.abs() + dy.abs(),         // Diamond (L1 norm)
        }
    }

    fn params_list() -> &'static [ParamSpec] {
        static PARAMS: &[ParamSpec] = &[
            ParamSpec {
                key: "speed",
                label: "Speed",
                kind: ParamKind::Float { min: 0.5, max: 5.0, step: 0.25 },
                default: ParamValue::Float(2.0),
            },
            ParamSpec {
                key: "depth",
                label: "Depth",
                kind: ParamKind::Float { min: 5.0, max: 50.0, step: 5.0 },
                default: ParamValue::Float(20.0),
            },
            ParamSpec {
                key: "texture",
                label: "Texture",
                kind: ParamKind::Enum { variants: TEXTURE_VARIANTS },
                default: ParamValue::Enum(0),
            },
            ParamSpec {
                key: "shape",
                label: "Shape",
                kind: ParamKind::Enum { variants: SHAPE_VARIANTS },
                default: ParamValue::Enum(0),
            },
            ParamSpec {
                key: "palette",
                label: "Palette",
                kind: ParamKind::Enum { variants: palette::PALETTE_NAMES },
                default: ParamValue::Enum(2),
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

impl Visual for Tunnel {
    fn name(&self) -> &'static str {
        "Tunnel"
    }

    fn params(&self) -> &[ParamSpec] {
        Self::params_list()
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<(), ParamError> {
        match key {
            "speed" => {
                if let ParamValue::Float(v) = value {
                    self.speed = v.clamp(0.5, 5.0);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            "depth" => {
                if let ParamValue::Float(v) = value {
                    self.depth_factor = v.clamp(5.0, 50.0);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            "texture" => {
                if let ParamValue::Enum(idx) = value {
                    if idx < TEXTURE_VARIANTS.len() {
                        self.texture_idx = idx;
                        Ok(())
                    } else {
                        Err(ParamError::new("invalid texture index"))
                    }
                } else {
                    Err(ParamError::type_mismatch(key, "enum"))
                }
            }
            "shape" => {
                if let ParamValue::Enum(idx) = value {
                    if idx < SHAPE_VARIANTS.len() {
                        self.shape_idx = idx;
                        Ok(())
                    } else {
                        Err(ParamError::new("invalid shape index"))
                    }
                } else {
                    Err(ParamError::type_mismatch(key, "enum"))
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
            "speed" => Some(ParamValue::Float(self.speed)),
            "depth" => Some(ParamValue::Float(self.depth_factor)),
            "texture" => Some(ParamValue::Enum(self.texture_idx)),
            "shape" => Some(ParamValue::Enum(self.shape_idx)),
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

        for yi in 0..h {
            for xi in 0..w {
                // Correct for aspect ratio
                let dx = (xi as f32 - cx) / 2.0;
                let dy = yi as f32 - cy;

                let dist = self.distance_from_center(dx, dy);
                if dist < 0.1 {
                    fb.set_cell(xi, yi, ' ', Color::BLACK, Color::BLACK);
                    continue;
                }

                // Compute tunnel coordinates
                let depth = self.depth_factor / dist;
                let angle = dy.atan2(dx);

                // Texture coordinates with time animation
                let u = math::fract(depth + self.time);
                let v = math::fract(angle / TAU + self.time * 0.3);

                // Sample texture
                let tex = self.sample_texture(u, v);

                // Color from palette with depth fog
                let fog = (1.0 - 1.0 / (depth * 0.2 + 1.0)).clamp(0.0, 1.0);
                let brightness = tex * (1.0 - fog * 0.5);

                let color_idx = ((brightness * 255.0) as usize + (self.time * 30.0) as usize) % 256;
                let mut color = self.palette[color_idx];
                
                // Apply fog
                color = Color::lerp(color, Color::BLACK, fog * 0.3);

                let ch = ramp::ramp(brightness, ramp::RAMP_EXTENDED);
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
        let mut effect = Tunnel::new(12345);
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
