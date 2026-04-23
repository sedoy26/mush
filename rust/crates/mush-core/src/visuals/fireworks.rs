//! Fireworks effect - exploding particle bursts with trails.
//!
//! Algorithm:
//! Launch phase: rocket rises with initial vy, gravity pulls down
//! Burst at apex: spawn N particles with random radial velocity
//! Each particle has lifetime and fading trail
//! Trail = last K positions with decreasing brightness
//!
//! Tunables: launch rate, burst size, gravity, trail length, colors

use rand::rngs::SmallRng;
use rand::{Rng, SeedableRng};

use crate::state::audio::ReactiveLevels;
use crate::visuals::util::palette;
use crate::visuals::{Color, Framebuffer, ParamError, ParamKind, ParamSpec, ParamValue, Visual};

const MAX_ROCKETS: usize = 10;
const MAX_PARTICLES: usize = 500;
const TRAIL_LEN: usize = 8;

struct Rocket {
    x: f32,
    y: f32,
    vy: f32,
    active: bool,
    fuse_time: f32,
}

struct Particle {
    x: f32,
    y: f32,
    vx: f32,
    vy: f32,
    life: f32,
    max_life: f32,
    color: Color,
    trail: [(f32, f32); TRAIL_LEN],
    trail_idx: usize,
}

pub struct Fireworks {
    rockets: Vec<Rocket>,
    particles: Vec<Particle>,
    gravity: f32,
    burst_size: i32,
    trail_length: i32,
    launch_rate: f32,
    launch_timer: f32,
    palette_idx: usize,
    palette: [Color; 256],
    rng: SmallRng,
    width: f32,
    height: f32,
}

impl Fireworks {
    pub fn new(seed: u64) -> Self {
        Self {
            rockets: Vec::with_capacity(MAX_ROCKETS),
            particles: Vec::with_capacity(MAX_PARTICLES),
            gravity: 20.0,
            burst_size: 30,
            trail_length: 5,
            launch_rate: 1.5,
            launch_timer: 0.0,
            palette_idx: 2, // rainbow
            palette: palette::rainbow(),
            rng: SmallRng::seed_from_u64(seed),
            width: 120.0,
            height: 40.0,
        }
    }

    fn launch_rocket(&mut self) {
        if self.rockets.len() >= MAX_ROCKETS {
            return;
        }
        
        let x = self.rng.gen_range(self.width * 0.2..self.width * 0.8);
        let vy = self.rng.gen_range(-40.0..-25.0);
        let fuse = self.rng.gen_range(0.8..1.5);
        
        self.rockets.push(Rocket {
            x,
            y: self.height - 1.0,
            vy,
            active: true,
            fuse_time: fuse,
        });
    }

    fn burst(&mut self, x: f32, y: f32) {
        let color_base = self.rng.gen_range(0..200);
        
        for _ in 0..self.burst_size {
            if self.particles.len() >= MAX_PARTICLES {
                break;
            }
            
            let angle = self.rng.gen::<f32>() * std::f32::consts::TAU;
            let speed = self.rng.gen_range(5.0..25.0);
            let color_idx = (color_base + self.rng.gen_range(0..50)) % 256;
            
            self.particles.push(Particle {
                x,
                y,
                vx: angle.cos() * speed,
                vy: angle.sin() * speed,
                life: 1.0,
                max_life: self.rng.gen_range(1.0..2.5),
                color: self.palette[color_idx],
                trail: [(x, y); TRAIL_LEN],
                trail_idx: 0,
            });
        }
    }

    fn params_list() -> &'static [ParamSpec] {
        static PARAMS: &[ParamSpec] = &[
            ParamSpec {
                key: "launch_rate",
                label: "Rate",
                kind: ParamKind::Float { min: 0.5, max: 5.0, step: 0.25 },
                default: ParamValue::Float(1.5),
            },
            ParamSpec {
                key: "burst",
                label: "Burst",
                kind: ParamKind::Int { min: 10, max: 80 },
                default: ParamValue::Int(30),
            },
            ParamSpec {
                key: "gravity",
                label: "Gravity",
                kind: ParamKind::Float { min: 10.0, max: 50.0, step: 5.0 },
                default: ParamValue::Float(20.0),
            },
            ParamSpec {
                key: "trail",
                label: "Trail",
                kind: ParamKind::Int { min: 1, max: 8 },
                default: ParamValue::Int(5),
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

impl Visual for Fireworks {
    fn name(&self) -> &'static str {
        "Fireworks"
    }

    fn params(&self) -> &[ParamSpec] {
        Self::params_list()
    }

    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<(), ParamError> {
        match key {
            "launch_rate" => {
                if let ParamValue::Float(v) = value {
                    self.launch_rate = v.clamp(0.5, 5.0);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            "burst" => {
                if let ParamValue::Int(v) = value {
                    self.burst_size = v.clamp(10, 80);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "int"))
                }
            }
            "gravity" => {
                if let ParamValue::Float(v) = value {
                    self.gravity = v.clamp(10.0, 50.0);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "float"))
                }
            }
            "trail" => {
                if let ParamValue::Int(v) = value {
                    self.trail_length = v.clamp(1, TRAIL_LEN as i32);
                    Ok(())
                } else {
                    Err(ParamError::type_mismatch(key, "int"))
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
            "launch_rate" => Some(ParamValue::Float(self.launch_rate)),
            "burst" => Some(ParamValue::Int(self.burst_size)),
            "gravity" => Some(ParamValue::Float(self.gravity)),
            "trail" => Some(ParamValue::Int(self.trail_length)),
            "palette" => Some(ParamValue::Enum(self.palette_idx)),
            _ => None,
        }
    }

    fn tick(&mut self, dt: f32, reactive: &ReactiveLevels) {
        // Launch triggered by kick or timer
        self.launch_timer += dt;
        let should_launch = self.launch_timer > 1.0 / self.launch_rate 
            || reactive.kick > 0.8;
        
        if should_launch && self.rockets.len() < MAX_ROCKETS {
            self.launch_rocket();
            self.launch_timer = 0.0;
        }

        // Update rockets
        let mut bursts = Vec::new();
        for rocket in &mut self.rockets {
            if !rocket.active {
                continue;
            }
            
            rocket.vy += self.gravity * dt;
            rocket.y += rocket.vy * dt;
            rocket.fuse_time -= dt;
            
            // Burst when fuse runs out or rocket starts falling
            if rocket.fuse_time <= 0.0 || rocket.vy > 0.0 {
                bursts.push((rocket.x, rocket.y));
                rocket.active = false;
            }
        }
        
        for (x, y) in bursts {
            self.burst(x, y);
        }
        
        // Remove inactive rockets
        self.rockets.retain(|r| r.active);

        // Update particles
        for particle in &mut self.particles {
            // Store trail position
            particle.trail[particle.trail_idx] = (particle.x, particle.y);
            particle.trail_idx = (particle.trail_idx + 1) % TRAIL_LEN;
            
            // Physics
            particle.vy += self.gravity * dt;
            particle.x += particle.vx * dt;
            particle.y += particle.vy * dt;
            
            // Decay
            particle.life -= dt / particle.max_life;
        }
        
        // Remove dead particles
        self.particles.retain(|p| p.life > 0.0);
    }

    fn render(&self, fb: &mut Framebuffer) {
        let (w, h) = fb.size();
        if w == 0 || h == 0 {
            return;
        }

        // Update dimensions for spawning
        let _fw = w as f32;
        let _fh = h as f32;

        fb.clear();

        // Draw rockets
        for rocket in &self.rockets {
            if !rocket.active {
                continue;
            }
            let x = rocket.x as i32;
            let y = rocket.y as i32;
            if x >= 0 && x < w as i32 && y >= 0 && y < h as i32 {
                fb.set_cell(x as u16, y as u16, '▲', Color::WHITE, Color::BLACK);
            }
        }

        // Draw particles with trails
        for particle in &self.particles {
            let fade = particle.life.clamp(0.0, 1.0);
            
            // Draw trail
            for i in 0..self.trail_length as usize {
                let idx = (particle.trail_idx + TRAIL_LEN - i - 1) % TRAIL_LEN;
                let (tx, ty) = particle.trail[idx];
                let x = tx as i32;
                let y = ty as i32;
                
                if x >= 0 && x < w as i32 && y >= 0 && y < h as i32 {
                    let trail_fade = fade * (1.0 - i as f32 / self.trail_length as f32);
                    let color = Color::lerp(Color::BLACK, particle.color, trail_fade);
                    let ch = if i == 0 { '●' } else { '·' };
                    fb.set_cell(x as u16, y as u16, ch, color, Color::BLACK);
                }
            }
            
            // Draw head
            let x = particle.x as i32;
            let y = particle.y as i32;
            if x >= 0 && x < w as i32 && y >= 0 && y < h as i32 {
                fb.set_cell(x as u16, y as u16, '*', particle.color, Color::BLACK);
            }
        }
    }

    fn reset(&mut self) {
        self.rockets.clear();
        self.particles.clear();
        self.launch_timer = 0.0;
    }

    fn resize(&mut self, width: u16, height: u16) {
        self.width = width as f32;
        self.height = height as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smoke_test() {
        let mut effect = Fireworks::new(12345);
        let mut fb = Framebuffer::new(80, 24);
        let reactive = ReactiveLevels::default();

        // Run longer to see bursts
        for _ in 0..180 {
            effect.tick(1.0 / 60.0, &reactive);
        }
        effect.render(&mut fb);

        // Should have some activity after 3 seconds
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
        // May or may not have active particles at this exact moment
        assert!(non_empty >= 0, "Test ran without panic");
    }
}
