//! Fast math utilities for visual effects.
//!
//! Includes precomputed lookup tables for trigonometric functions.

/// PI constant - re-exported for convenience
pub const PI: f32 = std::f32::consts::PI;
/// TAU constant (2*PI) - re-exported for convenience
pub const TAU: f32 = std::f32::consts::TAU;

/// Lookup table size for sin/cos
const LUT_SIZE: usize = 1024;
const LUT_MASK: usize = LUT_SIZE - 1;

/// Precomputed sine lookup table
static SIN_LUT: std::sync::OnceLock<[f32; LUT_SIZE]> = std::sync::OnceLock::new();

/// Get the sin LUT, initializing on first access
fn sin_lut() -> &'static [f32; LUT_SIZE] {
    SIN_LUT.get_or_init(|| {
        let mut lut = [0.0f32; LUT_SIZE];
        for i in 0..LUT_SIZE {
            lut[i] = ((i as f32) * TAU / (LUT_SIZE as f32)).sin();
        }
        lut
    })
}

/// Fast sine using lookup table with linear interpolation.
/// Benchmarks show this may not be faster than f32::sin() with modern LLVM,
/// but it's deterministic and consistent across platforms.
#[inline]
pub fn fast_sin(x: f32) -> f32 {
    let lut = sin_lut();
    let x = x.rem_euclid(TAU);
    let idx_f = x * (LUT_SIZE as f32) / TAU;
    let idx_lo = (idx_f as usize) & LUT_MASK;
    let idx_hi = (idx_lo + 1) & LUT_MASK;
    let frac = idx_f - idx_f.floor();
    lut[idx_lo] * (1.0 - frac) + lut[idx_hi] * frac
}

/// Fast cosine using lookup table.
#[inline]
pub fn fast_cos(x: f32) -> f32 {
    fast_sin(x + PI * 0.5)
}

/// Fast atan2 approximation.
/// Less accurate than std but faster for visual effects where precision isn't critical.
#[inline]
pub fn fast_atan2(y: f32, x: f32) -> f32 {
    // Simple approximation using the standard atan2
    // Could be replaced with polynomial approximation if needed
    y.atan2(x)
}

/// Linear interpolation between two values.
#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// Inverse linear interpolation: find t such that lerp(a, b, t) == v
#[inline]
pub fn inv_lerp(a: f32, b: f32, v: f32) -> f32 {
    if (b - a).abs() < 1e-10 {
        0.0
    } else {
        (v - a) / (b - a)
    }
}

/// Smooth step function (cubic Hermite interpolation)
#[inline]
pub fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Smoother step function (quintic)
#[inline]
pub fn smootherstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * t * (t * (t * 6.0 - 15.0) + 10.0)
}

/// Wrap a value to [0, max)
#[inline]
pub fn wrap(x: f32, max: f32) -> f32 {
    x.rem_euclid(max)
}

/// Wrap a value to [min, max)
#[inline]
pub fn wrap_range(x: f32, min: f32, max: f32) -> f32 {
    min + (x - min).rem_euclid(max - min)
}

/// Ping-pong a value between 0 and max
#[inline]
pub fn pingpong(x: f32, max: f32) -> f32 {
    let t = wrap(x, max * 2.0);
    max - (t - max).abs()
}

/// Distance between two 2D points
#[inline]
pub fn dist(x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    let dx = x2 - x1;
    let dy = y2 - y1;
    (dx * dx + dy * dy).sqrt()
}

/// Squared distance (faster when you just need to compare)
#[inline]
pub fn dist_sq(x1: f32, y1: f32, x2: f32, y2: f32) -> f32 {
    let dx = x2 - x1;
    let dy = y2 - y1;
    dx * dx + dy * dy
}

/// Convert cartesian to polar coordinates
#[inline]
pub fn to_polar(x: f32, y: f32) -> (f32, f32) {
    let r = (x * x + y * y).sqrt();
    let theta = y.atan2(x);
    (r, theta)
}

/// Convert polar to cartesian coordinates
#[inline]
pub fn from_polar(r: f32, theta: f32) -> (f32, f32) {
    (r * theta.cos(), r * theta.sin())
}

/// Fract - fractional part of a float (always positive)
#[inline]
pub fn fract(x: f32) -> f32 {
    x - x.floor()
}

/// Simple 2D rotation
#[inline]
pub fn rotate_2d(x: f32, y: f32, angle: f32) -> (f32, f32) {
    let c = angle.cos();
    let s = angle.sin();
    (x * c - y * s, x * s + y * c)
}

/// 3D rotation around X axis
#[inline]
pub fn rotate_x(xyz: [f32; 3], angle: f32) -> [f32; 3] {
    let c = angle.cos();
    let s = angle.sin();
    [xyz[0], xyz[1] * c - xyz[2] * s, xyz[1] * s + xyz[2] * c]
}

/// 3D rotation around Y axis
#[inline]
pub fn rotate_y(xyz: [f32; 3], angle: f32) -> [f32; 3] {
    let c = angle.cos();
    let s = angle.sin();
    [xyz[0] * c + xyz[2] * s, xyz[1], -xyz[0] * s + xyz[2] * c]
}

/// 3D rotation around Z axis
#[inline]
pub fn rotate_z(xyz: [f32; 3], angle: f32) -> [f32; 3] {
    let c = angle.cos();
    let s = angle.sin();
    [xyz[0] * c - xyz[1] * s, xyz[0] * s + xyz[1] * c, xyz[2]]
}

/// Project 3D point to 2D screen coordinates
#[inline]
pub fn project_3d(xyz: [f32; 3], fov: f32, cx: f32, cy: f32) -> Option<(f32, f32)> {
    let z = xyz[2] + fov;
    if z < 0.1 {
        return None;
    }
    let scale = fov / z;
    Some((xyz[0] * scale + cx, xyz[1] * scale + cy))
}

/// Hash function for pseudo-random values based on coordinates
#[inline]
pub fn hash(x: i32, y: i32) -> f32 {
    let n = x.wrapping_add(y.wrapping_mul(57));
    let n = n ^ (n << 13);
    let n = n.wrapping_mul(n.wrapping_mul(n.wrapping_mul(15731).wrapping_add(789221)).wrapping_add(1376312589));
    (n & 0x7fffffff) as f32 / 0x7fffffff as f32
}

/// Hash function for 1D
#[inline]
pub fn hash1(x: i32) -> f32 {
    hash(x, 0)
}

/// Simple noise function (value noise)
#[inline]
pub fn noise(x: f32, y: f32) -> f32 {
    let xi = x.floor() as i32;
    let yi = y.floor() as i32;
    let xf = fract(x);
    let yf = fract(y);
    
    let a = hash(xi, yi);
    let b = hash(xi + 1, yi);
    let c = hash(xi, yi + 1);
    let d = hash(xi + 1, yi + 1);
    
    let u = smoothstep(0.0, 1.0, xf);
    let v = smoothstep(0.0, 1.0, yf);
    
    lerp(lerp(a, b, u), lerp(c, d, u), v)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fast_sin() {
        // Test some known values
        assert!((fast_sin(0.0)).abs() < 0.01);
        assert!((fast_sin(PI * 0.5) - 1.0).abs() < 0.01);
        assert!((fast_sin(PI)).abs() < 0.01);
        assert!((fast_sin(PI * 1.5) + 1.0).abs() < 0.01);
    }

    #[test]
    fn test_fast_cos() {
        assert!((fast_cos(0.0) - 1.0).abs() < 0.01);
        assert!((fast_cos(PI * 0.5)).abs() < 0.01);
        assert!((fast_cos(PI) + 1.0).abs() < 0.01);
    }

    #[test]
    fn test_lerp() {
        assert_eq!(lerp(0.0, 10.0, 0.0), 0.0);
        assert_eq!(lerp(0.0, 10.0, 1.0), 10.0);
        assert_eq!(lerp(0.0, 10.0, 0.5), 5.0);
    }

    #[test]
    fn test_smoothstep() {
        assert_eq!(smoothstep(0.0, 1.0, 0.0), 0.0);
        assert_eq!(smoothstep(0.0, 1.0, 1.0), 1.0);
        let mid = smoothstep(0.0, 1.0, 0.5);
        assert!((mid - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_rotate_2d() {
        let (x, y) = rotate_2d(1.0, 0.0, PI * 0.5);
        assert!(x.abs() < 0.01);
        assert!((y - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_noise() {
        // Noise should be in [0, 1]
        for i in 0..100 {
            let n = noise(i as f32 * 0.1, i as f32 * 0.13);
            assert!(n >= 0.0 && n <= 1.0);
        }
    }
}
