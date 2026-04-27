//! Spinning Donut - Andy Sloane's classic ASCII torus renderer.
//!
//! Algorithm:
//! Parametric torus: (θ, φ) both in [0, 2π)
//! For each point: circle_x = R2 + R1·cos θ, circle_y = R1·sin θ
//! Rotate by angles A (X-axis) and B (Z-axis)
//! Project: ooz = 1/(K2+z), screen (xp, yp) = (cx + K1·ooz·x', cy - K1·ooz·y'/2)
//! Z-buffer: only draw if ooz > zbuf[yp][xp]
//! Lambertian shading, index into ramp " .,-~:;=!*#$@"
//!
//! Tunables: R1, R2, A-speed, B-speed, palette

use crate::state::audio::ReactiveLevels;
use crate::visuals::util::ramp;
use crate::visuals::{Color, Framebuffer, ParamError, ParamKind, ParamSpec, ParamValue, Visual};

use rand::rngs::SmallRng;
use rand::SeedableRng;

const MAX_WIDTH: usize = 200;
const MAX_HEIGHT: usize = 80;

pub struct Donut {
    a: f32, // Rotation around X
    b: f32, // Rotation around Z
    r1: f32, // Tube radius
    r2: f32, // Torus major radius (base)
    /// Major radius used for drawing this frame (base + kick * kick_swell), updated in tick.
    r2_draw: f32,
    /// How much `r2` grows when kick envelope is hot (VISUALS: Kick swell).
    kick_swell: f32,
    a_speed: f32,
    b_speed: f32,
    k1: f32, // Screen scaling
    k2: f32, // Distance from viewer
    base_color: (u8, u8, u8), // Theme-based base color
    #[allow(dead_code)]
    rng: SmallRng,
}

impl Donut {
    pub fn new(seed: u64) -> Self {
        let r2 = 2.0;
        Self {
            a: 0.0,
            b: 0.0,
            r1: 1.0,
            r2,
            r2_draw: r2,
            kick_swell: 0.45,
            a_speed: 1.0,
            b_speed: 0.5,
            k1: 30.0,
            k2: 5.0,
            base_color: (200, 100, 220), // Default magenta
            rng: SmallRng::seed_from_u64(seed),
        }
    }

    fn params_list() -> &'static [ParamSpec] {
        static PARAMS: &[ParamSpec] = &[
            ParamSpec {
                key: "r1",
                label: "Tube Rad",
                kind: ParamKind::Float { min: 0.3, max: 2.0, step: 0.1 },
                default: ParamValue::Float(1.0),
            },
            ParamSpec {
                key: "r2",
                label: "Torus Rad",
                kind: ParamKind::Float { min: 1.0, max: 4.0, step: 0.1 },
                default: ParamValue::Float(2.0),
            },
            ParamSpec {
                key: "kick_swell",
                label: "Kick swell",
                kind: ParamKind::Float {
                    min: 0.0,
                    max: 1.8,
                    step: 0.05,
                },
                default: ParamValue::Float(0.45),
            },
            ParamSpec {
                key: "a_speed",
                label: "X Spin",
                kind: ParamKind::Float { min: 0.0, max: 3.0, step: 0.1 },
                default: ParamValue::Float(1.0),
            },
            ParamSpec {
                key: "b_speed",
                label: "Z Spin",
                kind: ParamKind::Float { min: 0.0, max: 3.0, step: 0.1 },
                default: ParamValue::Float(0.5),
            },
        ];
        PARAMS
    }
}

impl Visual for Donut {
    fn name(&self) -> &'static str {
        "Donut"
    }

    fn params(&self) -> &[ParamSpec] {
        Self::params_list()
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<(), ParamError> {
        match key {
            "r1" => {
                if let ParamValue::Float(v) = value {
                    self.r1 = v.clamp(0.3, 2.0);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            "r2" => {
                if let ParamValue::Float(v) = value {
                    self.r2 = v.clamp(1.0, 4.0);
                    self.r2_draw = self.r2;
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            "kick_swell" => {
                if let ParamValue::Float(v) = value {
                    self.kick_swell = v.clamp(0.0, 1.8);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            "a_speed" => {
                if let ParamValue::Float(v) = value {
                    self.a_speed = v.clamp(0.0, 3.0);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            "b_speed" => {
                if let ParamValue::Float(v) = value {
                    self.b_speed = v.clamp(0.0, 3.0);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            _ => Err(ParamError::unknown_key(key)),
        }
    }

    fn get_param(&self, key: &str) -> Option<ParamValue> {
        match key {
            "r1" => Some(ParamValue::Float(self.r1)),
            "r2" => Some(ParamValue::Float(self.r2)),
            "kick_swell" => Some(ParamValue::Float(self.kick_swell)),
            "a_speed" => Some(ParamValue::Float(self.a_speed)),
            "b_speed" => Some(ParamValue::Float(self.b_speed)),
            _ => None,
        }
    }

    fn tick(&mut self, dt: f32, reactive: &ReactiveLevels) {
        // Audio-reactive speed: faster on kick/snare, baseline from master
        let speed_mod = 1.0 + reactive.master * 0.3 + reactive.kick * 0.5 + reactive.snare * 0.3;
        self.a += dt * self.a_speed * speed_mod;
        self.b += dt * self.b_speed * speed_mod;
        // Torus major radius swells on kick (amount set via "Kick swell" / kick_swell param).
        let kick = reactive.kick.clamp(0.0, 1.0);
        self.r2_draw = (self.r2 + kick * self.kick_swell).clamp(1.0, 6.0);
    }

    fn render(&self, fb: &mut Framebuffer) {
        let (w, h) = fb.size();
        if w == 0 || h == 0 {
            return;
        }

        fb.clear();

        // Initialize z-buffer
        let mut zbuffer = [[0.0f32; MAX_WIDTH]; MAX_HEIGHT];
        let w = w.min(MAX_WIDTH as u16) as usize;
        let h = h.min(MAX_HEIGHT as u16) as usize;

        let cx = w as f32 / 2.0;
        let cy = h as f32 / 2.0;

        // Precompute trig
        let sin_a = self.a.sin();
        let cos_a = self.a.cos();
        let sin_b = self.b.sin();
        let cos_b = self.b.cos();

        // Sample the torus surface
        let theta_steps = 120;
        let phi_steps = 60;
        let theta_spacing = std::f32::consts::TAU / theta_steps as f32;
        let phi_spacing = std::f32::consts::TAU / phi_steps as f32;

        for i in 0..theta_steps {
            let theta = i as f32 * theta_spacing;
            let cos_theta = theta.cos();
            let sin_theta = theta.sin();

            for j in 0..phi_steps {
                let phi = j as f32 * phi_spacing;
                let cos_phi = phi.cos();
                let sin_phi = phi.sin();

                // Circle in XZ plane
                let circle_x = self.r2 + self.r1 * cos_theta;
                let circle_y = self.r1 * sin_theta;

                // Rotate around X by A
                let _x = circle_x;
                let _y = circle_y * cos_a - sin_a * circle_x * 0.0;
                let _z = circle_y * sin_a + cos_a * circle_x * 0.0;

                // Actually, let's do this properly
                // Torus parametric: 
                // x = (R2 + R1*cos(theta)) * cos(phi)
                // y = (R2 + R1*cos(theta)) * sin(phi) 
                // z = R1 * sin(theta)

                let x = (self.r2_draw + self.r1 * cos_theta) * cos_phi;
                let y = (self.r2_draw + self.r1 * cos_theta) * sin_phi;
                let z = self.r1 * sin_theta;

                // Rotate around X by A
                let y1 = y * cos_a - z * sin_a;
                let z1 = y * sin_a + z * cos_a;

                // Rotate around Z by B
                let x2 = x * cos_b - y1 * sin_b;
                let y2 = x * sin_b + y1 * cos_b;
                let z2 = z1;

                // Move away from viewer
                let z_trans = z2 + self.k2;
                if z_trans < 0.1 {
                    continue;
                }

                // Project
                let ooz = 1.0 / z_trans;
                let xp = (cx + self.k1 * ooz * x2) as i32;
                let yp = (cy - self.k1 * ooz * y2 * 0.5) as i32; // *0.5 for aspect ratio

                if xp < 0 || xp >= w as i32 || yp < 0 || yp >= h as i32 {
                    continue;
                }

                let xpi = xp as usize;
                let ypi = yp as usize;

                // Z-buffer test
                if ooz <= zbuffer[ypi][xpi] {
                    continue;
                }
                zbuffer[ypi][xpi] = ooz;

                // Calculate surface normal for lighting
                // Normal = (cos_theta * cos_phi, cos_theta * sin_phi, sin_theta)
                // Then rotate same as vertex

                let nx = cos_theta * cos_phi;
                let ny = cos_theta * sin_phi;
                let nz = sin_theta;

                // Rotate normal around X by A
                let ny1 = ny * cos_a - nz * sin_a;
                let nz1 = ny * sin_a + nz * cos_a;

                // Rotate around Z by B
                let _nx2 = nx * cos_b - ny1 * sin_b;
                let ny2 = nx * sin_b + ny1 * cos_b;
                let nz2 = nz1;

                // Light direction: toward viewer and slightly up-left
                // Simplified: dot product with (0, 0.7, -0.7)
                let luminance = ny2 * 0.707 - nz2 * 0.707;
                let luminance = luminance.clamp(0.0, 1.0);

                let ch = ramp::ramp(luminance, ramp::RAMP_DONUT);
                // Use theme-based base color modulated by luminance
                let (br, bg, bb) = self.base_color;
                let r = (br as f32 * luminance).min(255.0) as u8;
                let g = (bg as f32 * luminance).min(255.0) as u8;
                let b = (bb as f32 * luminance).min(255.0) as u8;
                let color = Color::rgb(r, g, b);

                fb.set_cell(xp as u16, yp as u16, ch, color, Color::BLACK);
            }
        }
    }

    fn reset(&mut self) {
        self.a = 0.0;
        self.b = 0.0;
        self.r2_draw = self.r2;
    }

    fn set_base_color(&mut self, r: u8, g: u8, b: u8) {
        self.base_color = (r, g, b);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_test() {
        let mut effect = Donut::new(12345);
        let mut fb = Framebuffer::new(80, 24);
        let reactive = ReactiveLevels::default();

        for _ in 0..60 {
            effect.tick(1.0 / 60.0, &reactive);
        }
        effect.render(&mut fb);

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
        assert!(non_empty >= 50, "Expected donut to be rendered");
    }
}
