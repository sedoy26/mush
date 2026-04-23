//! Pluggable visual effects system for mush terminal synthesizer.
//!
//! Effects implement the `Visual` trait and are registered in the `VisualRegistry`.
//! All effects receive audio-reactive parameters and render to a `Framebuffer`.

mod framebuffer;
mod params;
pub mod util;

// Effects
mod plasma;
mod kaleidoscope;
mod matrix_rain;
mod fire;
mod tunnel;
mod donut;
mod fireworks;
mod ripples;
mod radio;
mod cube;

pub use framebuffer::{Cell, Color, Framebuffer};
pub use params::{ParamError, ParamKind, ParamSpec, ParamValue};

use crate::state::audio::ReactiveLevels;

/// A visual effect that can be rendered to the terminal framebuffer.
pub trait Visual: Send {
    /// Display name shown in UI when selecting effects
    fn name(&self) -> &'static str;

    /// List of tunable parameters exposed to the settings UI
    fn params(&self) -> &[ParamSpec];

    /// Modify a parameter by key
    fn set_param(&mut self, key: &str, value: ParamValue) -> Result<(), ParamError>;

    /// Get current parameter value
    fn get_param(&self, key: &str) -> Option<ParamValue>;

    /// Advance animation state by `dt` seconds
    fn tick(&mut self, dt: f32, reactive: &ReactiveLevels);

    /// Render current state to framebuffer
    fn render(&self, fb: &mut Framebuffer);

    /// Reset to initial state (e.g., when switching to this effect)
    fn reset(&mut self) {}

    /// Called when the target framebuffer size changes.
    /// Effects with internal buffers should resize them here.
    fn resize(&mut self, _width: u16, _height: u16) {}

    /// Set base color (for theme-aware effects like Donut)
    fn set_base_color(&mut self, _r: u8, _g: u8, _b: u8) {}
}

/// Registry of available visual effects.
pub struct VisualRegistry {
    visuals: Vec<Box<dyn Visual>>,
    current_index: usize,
}

impl Default for VisualRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl VisualRegistry {
    /// Create a new registry with all built-in effects registered.
    pub fn new() -> Self {
        let mut registry = Self {
            visuals: Vec::new(),
            current_index: 0,
        };
        
        // Register effects in display order
        registry.register(Box::new(plasma::Plasma::new(42)));
        registry.register(Box::new(kaleidoscope::Kaleidoscope::new(43)));
        registry.register(Box::new(matrix_rain::MatrixRain::new(44)));
        registry.register(Box::new(fire::Fire::new(45)));
        registry.register(Box::new(tunnel::Tunnel::new(46)));
        registry.register(Box::new(donut::Donut::new(47)));
        registry.register(Box::new(fireworks::Fireworks::new(48)));
        registry.register(Box::new(ripples::Ripples::new(49)));
        registry.register(Box::new(radio::Radio::new(50)));
        registry.register(Box::new(cube::Cube::new(51)));
        
        registry
    }

    /// Register a visual effect.
    pub fn register(&mut self, visual: Box<dyn Visual>) {
        self.visuals.push(visual);
    }

    /// Get the currently selected effect.
    pub fn current(&self) -> Option<&dyn Visual> {
        self.visuals.get(self.current_index).map(|v| v.as_ref())
    }

    /// Get mutable reference to the currently selected effect.
    pub fn current_mut(&mut self) -> Option<&mut Box<dyn Visual>> {
        self.visuals.get_mut(self.current_index)
    }

    /// Get the current effect's name.
    pub fn current_name(&self) -> &'static str {
        self.current().map_or("None", |v| v.name())
    }

    /// Select an effect by name. Returns true if found.
    /// Only resets if actually changing to a different effect.
    pub fn select_by_name(&mut self, name: &str) -> bool {
        for (i, visual) in self.visuals.iter().enumerate() {
            if visual.name() == name {
                if self.current_index != i {
                    self.current_index = i;
                    if let Some(v) = self.visuals.get_mut(i) {
                        v.reset();
                    }
                }
                return true;
            }
        }
        false
    }

    /// Cycle through effects. Positive delta goes forward.
    pub fn cycle(&mut self, delta: i32) {
        if self.visuals.is_empty() {
            return;
        }
        let len = self.visuals.len() as i32;
        let new_index = (self.current_index as i32 + delta).rem_euclid(len) as usize;
        if new_index != self.current_index {
            self.current_index = new_index;
            if let Some(v) = self.visuals.get_mut(new_index) {
                v.reset();
            }
        }
    }

    /// Get list of all effect names.
    pub fn names(&self) -> Vec<&'static str> {
        self.visuals.iter().map(|v| v.name()).collect()
    }

    /// Get current index.
    pub fn current_index(&self) -> usize {
        self.current_index
    }

    /// Set current index directly. Resets effect if changing.
    pub fn set_index(&mut self, idx: usize) {
        if idx < self.visuals.len() && idx != self.current_index {
            self.current_index = idx;
            if let Some(v) = self.visuals.get_mut(idx) {
                v.reset();
            }
        }
    }

    /// Get effect count.
    pub fn len(&self) -> usize {
        self.visuals.len()
    }

    /// Check if registry is empty.
    pub fn is_empty(&self) -> bool {
        self.visuals.is_empty()
    }
}
