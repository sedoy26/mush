//! 3D Rotating Cube effect - wireframe cube with perspective projection.
//!
//! Algorithm:
//! - 8 vertices, 12 edges
//! - Rotate around X, Y, Z axes animating over time
//! - Project 3D → 2D with perspective
//! - Draw edges with Bresenham line algorithm
//!
//! Tunables: rotation speed, cube size, line density, perspective, palette

use crate::state::audio::ReactiveLevels;
use crate::visuals::util::palette;
use crate::visuals::{Color, Framebuffer, ParamError, ParamKind, ParamSpec, ParamValue, Visual};

use rand::rngs::SmallRng;
use rand::SeedableRng;

pub struct Cube {
    time: f32,
    rotation_speed: f32,
    size: f32,
    perspective: f32,
    palette_idx: usize,
    palette: [Color; 256],
    #[allow(dead_code)]
    rng: SmallRng,
}

impl Cube {
    pub fn new(seed: u64) -> Self {
        Self {
            time: 0.0,
            rotation_speed: 1.0,
            size: 1.0,
            perspective: 3.0,
            palette_idx: 3, // matrix
            palette: palette::matrix_green(),
            rng: SmallRng::seed_from_u64(seed),
        }
    }

    fn params_list() -> &'static [ParamSpec] {
        static PARAMS: &[ParamSpec] = &[
            ParamSpec {
                key: "speed",
                label: "Speed",
                kind: ParamKind::Float { min: 0.1, max: 3.0, step: 0.1 },
                default: ParamValue::Float(1.0),
            },
            ParamSpec {
                key: "size",
                label: "Size",
                kind: ParamKind::Float { min: 0.5, max: 2.0, step: 0.1 },
                default: ParamValue::Float(1.0),
            },
            ParamSpec {
                key: "perspective",
                label: "Depth",
                kind: ParamKind::Float { min: 2.0, max: 6.0, step: 0.5 },
                default: ParamValue::Float(3.0),
            },
            ParamSpec {
                key: "palette",
                label: "Palette",
                kind: ParamKind::Enum { variants: palette::PALETTE_NAMES },
                default: ParamValue::Enum(3),
            },
        ];
        PARAMS
    }

    fn update_palette(&mut self) {
        if let Some(p) = palette::by_name(palette::PALETTE_NAMES[self.palette_idx]) {
            self.palette = p;
        }
    }

    /// Draw a line using Bresenham's algorithm
    fn draw_line(&self, fb: &mut Framebuffer, x1: i32, y1: i32, x2: i32, y2: i32, color: Color) {
        let (w, h) = fb.size();
        let dx = (x2 - x1).abs();
        let dy = (y2 - y1).abs();
        let sx = if x1 < x2 { 1 } else { -1 };
        let sy = if y1 < y2 { 1 } else { -1 };
        let mut err = dx - dy;
        let mut x = x1;
        let mut y = y1;

        loop {
            if x >= 0 && x < w as i32 && y >= 0 && y < h as i32 {
                fb.set_cell(x as u16, y as u16, '·', color, Color::BLACK);
            }
            if x == x2 && y == y2 {
                break;
            }
            let e2 = 2 * err;
            if e2 > -dy {
                err -= dy;
                x += sx;
            }
            if e2 < dx {
                err += dx;
                y += sy;
            }
        }
    }
}

impl Visual for Cube {
    fn name(&self) -> &'static str {
        "Cube"
    }

    fn params(&self) -> &[ParamSpec] {
        Self::params_list()
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<(), ParamError> {
        match key {
            "speed" => {
                if let ParamValue::Float(v) = value {
                    self.rotation_speed = v.clamp(0.1, 3.0);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            "size" => {
                if let ParamValue::Float(v) = value {
                    self.size = v.clamp(0.5, 2.0);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            "perspective" => {
                if let ParamValue::Float(v) = value {
                    self.perspective = v.clamp(2.0, 6.0);
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
            "speed" => Some(ParamValue::Float(self.rotation_speed)),
            "size" => Some(ParamValue::Float(self.size)),
            "perspective" => Some(ParamValue::Float(self.perspective)),
            "palette" => Some(ParamValue::Enum(self.palette_idx)),
            _ => None,
        }
    }

    fn tick(&mut self, dt: f32, reactive: &ReactiveLevels) {
        let speed_mod = 0.5 + reactive.master * 1.5;
        self.time += dt * self.rotation_speed * speed_mod;
    }

    fn render(&self, fb: &mut Framebuffer) {
        let (w, h) = fb.size();
        if w == 0 || h == 0 {
            return;
        }

        fb.clear();

        // Scale cube by size parameter
        let s = self.size;
        let vertices: [[f32; 3]; 8] = [
            [-s, -s, -s],
            [s, -s, -s],
            [s, s, -s],
            [-s, s, -s],
            [-s, -s, s],
            [s, -s, s],
            [s, s, s],
            [-s, s, s],
        ];

        // 12 edges connecting vertices
        const EDGES: [(usize, usize); 12] = [
            (0, 1), (1, 2), (2, 3), (3, 0), // Back face
            (4, 5), (5, 6), (6, 7), (7, 4), // Front face
            (0, 4), (1, 5), (2, 6), (3, 7), // Connecting edges
        ];

        // Rotation angles
        let rot_x = self.time;
        let rot_y = self.time * 0.7;
        let rot_z = self.time * 0.5;

        let cos_x = rot_x.cos();
        let sin_x = rot_x.sin();
        let cos_y = rot_y.cos();
        let sin_y = rot_y.sin();
        let cos_z = rot_z.cos();
        let sin_z = rot_z.sin();

        // Rotation function
        let rotate = |v: [f32; 3]| -> [f32; 3] {
            // Rotate around X
            let v = [v[0], v[1] * cos_x - v[2] * sin_x, v[1] * sin_x + v[2] * cos_x];
            // Rotate around Y
            let v = [v[0] * cos_y + v[2] * sin_y, v[1], -v[0] * sin_y + v[2] * cos_y];
            // Rotate around Z
            [v[0] * cos_z - v[1] * sin_z, v[0] * sin_z + v[1] * cos_z, v[2]]
        };

        // Perspective projection
        let cx = w as f32 / 2.0;
        let cy = h as f32 / 2.0;
        let project = |v: [f32; 3]| -> Option<(i32, i32)> {
            let z = v[2] + self.perspective;
            if z < 0.1 {
                return None;
            }
            let scale = 2.0 / z;
            // Account for terminal character aspect ratio (~2:1)
            let proj_x = (v[0] * scale * w as f32 / 4.0 + cx) as i32;
            let proj_y = (v[1] * scale * h as f32 / 4.0 + cy) as i32;
            Some((proj_x, proj_y))
        };

        // Transform vertices
        let rotated: Vec<_> = vertices.iter().map(|&v| rotate(v)).collect();

        // Calculate edge depths for sorting (back-to-front rendering)
        let mut edge_depths: Vec<(usize, f32)> = EDGES
            .iter()
            .enumerate()
            .map(|(i, &(a, b))| {
                let depth = (rotated[a][2] + rotated[b][2]) / 2.0;
                (i, depth)
            })
            .collect();
        edge_depths.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

        // Draw edges from back to front
        for (edge_idx, depth) in edge_depths {
            let (i, j) = EDGES[edge_idx];
            if let (Some((x1, y1)), Some((x2, y2))) = (project(rotated[i]), project(rotated[j])) {
                // Color varies by depth
                let depth_norm = ((depth + self.size) / (2.0 * self.size)).clamp(0.0, 1.0);
                let color_idx = (depth_norm * 200.0) as usize + 55;
                let color = self.palette[color_idx.min(255)];
                self.draw_line(fb, x1, y1, x2, y2, color);
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
        let mut effect = Cube::new(12345);
        let mut fb = Framebuffer::new(80, 24);
        let reactive = ReactiveLevels::default();

        for _ in 0..60 {
            effect.tick(1.0 / 60.0, &reactive);
        }
        effect.render(&mut fb);

        // Should have drawn some cube edges
        let mut non_empty = 0;
        for y in 0..24 {
            for x in 0..80 {
                if let Some(cell) = fb.get_cell(x, y) {
                    if cell.ch != ' ' {
                        non_empty += 1;
                    }
                }
            }
        }
        assert!(non_empty > 10, "Expected cube edges to be drawn");
    }
}
