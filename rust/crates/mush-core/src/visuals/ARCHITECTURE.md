# Visuals Module Architecture

This document describes the pluggable visual effects system for mush.

## Overview

The visuals module provides a trait-based system for rendering animated ASCII effects in the terminal. Effects are registered in a global registry and can be selected/cycled at runtime. All effects receive audio-reactive parameters for creating music-responsive animations.

## Core Trait

```rust
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
    
    /// Reset to initial state (e.g. when switching to this effect)
    fn reset(&mut self) {}
}
```

## Parameter System

Parameters enable live tuning of effects via the settings UI:

```rust
pub struct ParamSpec {
    pub key: &'static str,      // Internal identifier
    pub label: &'static str,    // UI display label
    pub kind: ParamKind,        // Type and constraints
    pub default: ParamValue,    // Initial value
}

pub enum ParamKind {
    Float { min: f32, max: f32, step: f32 },
    Int { min: i32, max: i32 },
    Bool,
    Enum { variants: &'static [&'static str] },
}

pub enum ParamValue {
    Float(f32),
    Int(i32),
    Bool(bool),
    Enum(usize),  // Index into variants
}

pub struct ParamError {
    pub message: String,
}
```

## Framebuffer Abstraction

Effects render to a `Framebuffer` that wraps the underlying terminal:

```rust
pub struct Framebuffer {
    width: u16,
    height: u16,
    cells: Vec<Cell>,
}

pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
}

#[derive(Clone, Copy)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Framebuffer {
    pub fn new(width: u16, height: u16) -> Self;
    pub fn size(&self) -> (u16, u16);
    pub fn clear(&mut self);
    pub fn set_cell(&mut self, x: u16, y: u16, ch: char, fg: Color, bg: Color);
    pub fn get_cell(&self, x: u16, y: u16) -> Option<&Cell>;
    
    // Convenience methods
    pub fn set_char(&mut self, x: u16, y: u16, ch: char);
    pub fn fill(&mut self, ch: char, fg: Color, bg: Color);
}
```

**Important**: Effects never call crossterm directly. The framebuffer is converted to terminal output by the main render loop.

## Color Handling

Since crossterm supports true color (RGB), we use full RGB colors internally:

```rust
impl Color {
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0 };
    pub const WHITE: Color = Color { r: 255, g: 255, b: 255 };
    
    pub fn from_hsv(h: f32, s: f32, v: f32) -> Color;
    pub fn lerp(a: Color, b: Color, t: f32) -> Color;
}
```

## Registry

Effects are registered at startup and accessed by name:

```rust
pub struct VisualRegistry {
    visuals: Vec<Box<dyn Visual>>,
    current_index: usize,
}

impl VisualRegistry {
    pub fn new() -> Self;
    pub fn register(&mut self, visual: Box<dyn Visual>);
    pub fn current(&self) -> &dyn Visual;
    pub fn current_mut(&mut self) -> &mut dyn Visual;
    pub fn select_by_name(&mut self, name: &str) -> bool;
    pub fn cycle(&mut self, delta: i32);
    pub fn names(&self) -> Vec<&'static str>;
}
```

## Utility Modules

### `util::palette`

Pre-computed 256-entry color LUTs:

```rust
pub type Palette = [Color; 256];

pub const FIRE: Palette = /* black → red → yellow → white */;
pub const ICE: Palette = /* dark blue → cyan → white */;
pub const RAINBOW: Palette = /* hue sweep at full saturation */;
pub const MATRIX_GREEN: Palette = /* black → dark green → bright green */;
pub const PLASMA: Palette = /* vibrant cycling colors */;
pub const MONO: Palette = /* grayscale */;
pub const AMBER: Palette = /* black → amber → white, CRT style */;

pub fn from_gradient(stops: &[(f32, Color)]) -> Palette;
```

### `util::ramp`

Character brightness ramps for ASCII shading:

```rust
pub const RAMP_CLASSIC: &[char] = &[' ', '.', ':', '-', '=', '+', '*', '#', '%', '@'];
pub const RAMP_DONUT: &[char] = &[' ', '.', ',', '-', '~', ':', ';', '=', '!', '*', '#', '$', '@'];
pub const RAMP_DOTS: &[char] = &[' ', '·', '∙', '•', '○', '●'];
pub const RAMP_BLOCKS: &[char] = &[' ', '░', '▒', '▓', '█'];
pub const RAMP_EXTENDED: &[char] = /* 90-character dense gradient */;

pub fn ramp(v: f32, chars: &[char]) -> char;  // v in [0,1]
```

### `util::math`

Precomputed trigonometry and helpers:

```rust
pub const SIN_LUT: [f32; 1024];
pub const COS_LUT: [f32; 1024];

pub fn fast_sin(x: f32) -> f32;  // Uses LUT with linear interpolation
pub fn fast_cos(x: f32) -> f32;
pub fn fast_atan2(y: f32, x: f32) -> f32;

pub fn lerp(a: f32, b: f32, t: f32) -> f32;
pub fn inv_lerp(a: f32, b: f32, v: f32) -> f32;
pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32;

pub const TAU: f32 = std::f32::consts::TAU;
pub const PI: f32 = std::f32::consts::PI;
```

### Random Number Generation

Each effect owns a seeded `SmallRng`:

```rust
use rand::SeedableRng;
use rand::rngs::SmallRng;

impl MyEffect {
    pub fn new(seed: u64) -> Self {
        Self {
            rng: SmallRng::seed_from_u64(seed),
            // ...
        }
    }
}
```

**Rule**: Never use `thread_rng()` in tick/render. Effects must be deterministic given a seed.

## Integration with Main Loop

The render loop in `main.rs` calls the visual system:

```rust
// In render():
let visual_lines = match state.ui.visual_mode {
    VisualMode::Scope => render_scope_braille(...),
    VisualMode::Camera => runtime.render_camera_ascii(...),
    VisualMode::Effect => {
        // Use visual registry
        let registry = runtime.visuals();
        registry.current().render(&mut fb);
        fb.to_strings()
    }
};
```

## Effect Implementation Guidelines

1. **Preallocate buffers** in `new()`, reuse in `tick()`/`render()`. No `Vec::new()` or `format!()` in hot paths.

2. **Deterministic rendering**: Given identical seed + dt sequence, produce identical frames.

3. **Aspect ratio compensation**: Terminal chars are ~2:1 (tall). Effects should halve Y coordinates or double X to render circular shapes as circles.

4. **Error handling**: No `.unwrap()` or `.expect()` in tick/render. Use `.unwrap_or()` or match expressions.

5. **Audio reactivity**: Use `reactive.kick`, `reactive.snare`, etc. for music-driven animation.

6. **Parameter ranges**: Clamp all params to valid ranges. Invalid input should fail gracefully.

## File Structure

```
src/visuals/
├── mod.rs           # Trait, registry, public exports
├── framebuffer.rs   # Framebuffer abstraction
├── params.rs        # ParamSpec, ParamValue, ParamError
├── util/
│   ├── mod.rs
│   ├── palette.rs   # Color palettes
│   ├── ramp.rs      # Character ramps
│   └── math.rs      # Fast math utilities
├── plasma.rs
├── kaleidoscope.rs
├── matrix_rain.rs
├── fire.rs
├── starfield.rs
├── tunnel.rs
├── donut.rs
├── shapes.rs
├── galaxy.rs
├── fireworks.rs
├── balls.rs
├── ripples.rs
├── radio.rs
└── horizon.rs
```

## Testing Strategy

Each effect has an in-file smoke test:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn smoke_test() {
        let mut effect = MyEffect::new(12345);
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
        assert!(unique_chars.len() >= 5, "Expected varied output");
    }
}
```

## Performance Requirements

- All effects must render at ≥30 FPS at 120×40 terminal size in release builds
- No heap allocations in `tick()` or `render()` hot paths
- `cargo clippy -D clippy::perf` must pass
