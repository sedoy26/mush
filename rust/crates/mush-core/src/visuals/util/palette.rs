//! Color palettes for visual effects.
//!
//! Each palette is a 256-entry lookup table for mapping values [0, 255] to colors.

use crate::visuals::Color;

/// A 256-color lookup table.
pub type Palette = [Color; 256];

/// Create palette from gradient stops.
/// Stops are (position, color) where position is 0.0-1.0.
pub fn from_gradient(stops: &[(f32, Color)]) -> Palette {
    let mut palette = [Color::BLACK; 256];
    if stops.is_empty() {
        return palette;
    }
    if stops.len() == 1 {
        palette.fill(stops[0].1);
        return palette;
    }

    // Sort stops by position
    let mut sorted: Vec<_> = stops.to_vec();
    sorted.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(std::cmp::Ordering::Equal));

    for i in 0..256 {
        let t = i as f32 / 255.0;
        
        // Find surrounding stops
        let mut lower_idx = 0;
        for (idx, (pos, _)) in sorted.iter().enumerate() {
            if *pos <= t {
                lower_idx = idx;
            }
        }
        
        let upper_idx = (lower_idx + 1).min(sorted.len() - 1);
        
        let (lower_pos, lower_color) = sorted[lower_idx];
        let (upper_pos, upper_color) = sorted[upper_idx];
        
        let local_t = if (upper_pos - lower_pos).abs() < 0.0001 {
            0.0
        } else {
            ((t - lower_pos) / (upper_pos - lower_pos)).clamp(0.0, 1.0)
        };
        
        palette[i] = Color::lerp(lower_color, upper_color, local_t);
    }
    
    palette
}

/// Fire palette: black -> red -> yellow -> white
pub fn fire() -> Palette {
    from_gradient(&[
        (0.0, Color::BLACK),
        (0.33, Color::rgb(180, 0, 0)),
        (0.66, Color::rgb(255, 150, 0)),
        (0.85, Color::rgb(255, 220, 100)),
        (1.0, Color::WHITE),
    ])
}

/// Ice palette: dark blue -> cyan -> white
pub fn ice() -> Palette {
    from_gradient(&[
        (0.0, Color::rgb(0, 0, 30)),
        (0.3, Color::rgb(0, 50, 150)),
        (0.6, Color::rgb(50, 150, 255)),
        (0.8, Color::rgb(150, 220, 255)),
        (1.0, Color::WHITE),
    ])
}

/// Rainbow palette: full hue sweep
pub fn rainbow() -> Palette {
    let mut palette = [Color::BLACK; 256];
    for i in 0..256 {
        palette[i] = Color::from_hsv(i as f32 / 256.0, 1.0, 1.0);
    }
    palette
}

/// Matrix green palette: black -> dark green -> bright green
pub fn matrix_green() -> Palette {
    from_gradient(&[
        (0.0, Color::BLACK),
        (0.2, Color::rgb(0, 30, 0)),
        (0.5, Color::rgb(0, 100, 0)),
        (0.8, Color::rgb(0, 200, 0)),
        (1.0, Color::rgb(150, 255, 150)),
    ])
}

/// Plasma palette: vibrant cycling colors
pub fn plasma() -> Palette {
    let mut palette = [Color::BLACK; 256];
    for i in 0..256 {
        let t = i as f32 / 256.0;
        // Create a smooth cycling pattern
        let r = ((t * std::f32::consts::PI * 2.0).sin() * 0.5 + 0.5) * 255.0;
        let g = ((t * std::f32::consts::PI * 2.0 + 2.094).sin() * 0.5 + 0.5) * 255.0;
        let b = ((t * std::f32::consts::PI * 2.0 + 4.189).sin() * 0.5 + 0.5) * 255.0;
        palette[i] = Color::rgb(r as u8, g as u8, b as u8);
    }
    palette
}

/// Monochrome grayscale
pub fn mono() -> Palette {
    let mut palette = [Color::BLACK; 256];
    for i in 0..256 {
        palette[i] = Color::rgb(i as u8, i as u8, i as u8);
    }
    palette
}

/// Amber CRT style: black -> amber -> white
pub fn amber() -> Palette {
    from_gradient(&[
        (0.0, Color::BLACK),
        (0.3, Color::rgb(80, 40, 0)),
        (0.6, Color::rgb(180, 100, 0)),
        (0.8, Color::rgb(255, 180, 50)),
        (1.0, Color::rgb(255, 230, 180)),
    ])
}

/// Ocean palette: deep blue -> teal -> seafoam
pub fn ocean() -> Palette {
    from_gradient(&[
        (0.0, Color::rgb(0, 10, 30)),
        (0.3, Color::rgb(0, 50, 100)),
        (0.5, Color::rgb(0, 120, 150)),
        (0.7, Color::rgb(50, 180, 180)),
        (1.0, Color::rgb(150, 255, 220)),
    ])
}

/// Sunset palette: purple -> orange -> yellow
pub fn sunset() -> Palette {
    from_gradient(&[
        (0.0, Color::rgb(30, 0, 50)),
        (0.3, Color::rgb(150, 0, 80)),
        (0.5, Color::rgb(255, 80, 50)),
        (0.7, Color::rgb(255, 150, 50)),
        (1.0, Color::rgb(255, 220, 100)),
    ])
}

/// Neon palette: dark with bright accent colors
pub fn neon() -> Palette {
    from_gradient(&[
        (0.0, Color::rgb(10, 0, 20)),
        (0.25, Color::rgb(255, 0, 100)),
        (0.5, Color::rgb(0, 255, 255)),
        (0.75, Color::rgb(100, 0, 255)),
        (1.0, Color::rgb(255, 255, 0)),
    ])
}

/// Named palette lookup
pub fn by_name(name: &str) -> Option<Palette> {
    match name {
        "fire" => Some(fire()),
        "ice" => Some(ice()),
        "rainbow" => Some(rainbow()),
        "matrix" | "matrix_green" => Some(matrix_green()),
        "plasma" => Some(plasma()),
        "mono" | "grayscale" => Some(mono()),
        "amber" => Some(amber()),
        "ocean" => Some(ocean()),
        "sunset" => Some(sunset()),
        "neon" => Some(neon()),
        _ => None,
    }
}

/// List of available palette names
pub const PALETTE_NAMES: &[&str] = &[
    "fire", "ice", "rainbow", "matrix", "plasma", "mono", "amber", "ocean", "sunset", "neon",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fire_palette() {
        let p = fire();
        // First entry should be black
        assert_eq!(p[0].r, 0);
        assert_eq!(p[0].g, 0);
        assert_eq!(p[0].b, 0);
        // Last entry should be white
        assert_eq!(p[255].r, 255);
        assert_eq!(p[255].g, 255);
        assert_eq!(p[255].b, 255);
    }

    #[test]
    fn test_rainbow_palette() {
        let p = rainbow();
        // Should have red at start
        assert!(p[0].r > 200);
        assert!(p[0].g < 50);
        // Should have variety
        let mut different = 0;
        for i in 1..256 {
            if p[i] != p[i - 1] {
                different += 1;
            }
        }
        assert!(different > 200, "Rainbow should have many color transitions");
    }

    #[test]
    fn test_by_name() {
        assert!(by_name("fire").is_some());
        assert!(by_name("nonexistent").is_none());
    }
}
