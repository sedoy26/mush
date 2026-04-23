//! Character ramps for ASCII shading.
//!
//! Ramps map brightness values [0.0, 1.0] to characters, from sparse/dim to dense/bright.

/// Classic ASCII shading ramp
pub const RAMP_CLASSIC: &[char] = &[' ', '.', ':', '-', '=', '+', '*', '#', '%', '@'];

/// Andy Sloane's donut ramp
pub const RAMP_DONUT: &[char] = &[' ', '.', ',', '-', '~', ':', ';', '=', '!', '*', '#', '$', '@'];

/// Dot density ramp
pub const RAMP_DOTS: &[char] = &[' ', '·', '∙', '•', '○', '●'];

/// Block element ramp
pub const RAMP_BLOCKS: &[char] = &[' ', '░', '▒', '▓', '█'];

/// Extended ASCII art ramp (90 characters, very fine gradation)
pub const RAMP_EXTENDED: &[char] = &[
    ' ', '.', '`', '-', '\'', ':', '_', ',', '^', '=', ';', '>', '<', '+', '!', 
    'r', 'c', '*', '/', 'z', '?', 's', 'L', 'T', 'v', ')', 'J', '7', '(', '|',
    'F', 'i', '{', 'C', '}', 'f', 'I', '3', '1', 't', 'l', 'u', '[', 'n', 'e',
    'o', 'Z', '5', 'Y', 'x', 'j', 'y', 'a', ']', '2', 'E', 'S', 'w', 'q', 'k',
    'P', '6', 'h', '9', 'd', '4', 'V', 'p', 'O', 'G', 'b', 'U', 'A', 'K', 'X',
    'H', 'm', '8', 'R', 'D', '#', '$', 'B', 'g', '0', 'M', 'N', 'W', 'Q', '%', '@',
];

/// Simple brightness ramp
pub const RAMP_SIMPLE: &[char] = &[' ', '.', '+', '*', '#', '@'];

/// Line-drawing ramp for effects that need direction
pub const RAMP_LINES: &[char] = &[' ', '.', '-', '/', '|', '\\', '+', '*', '#'];

/// Matrix-style characters (katakana-ish)
pub const RAMP_MATRIX: &[char] = &[
    ' ', '.', ':', '0', '1', 'ｱ', 'ｲ', 'ｳ', 'ｴ', 'ｵ', 'ｶ', 'ｷ', 'ｸ', 'ｹ', 'ｺ',
];

/// Binary digits
pub const RAMP_BINARY: &[char] = &[' ', '0', '1'];

/// Hex digits
pub const RAMP_HEX: &[char] = &[' ', '0', '1', '2', '3', '4', '5', '6', '7', '8', '9', 'A', 'B', 'C', 'D', 'E', 'F'];

/// Map a value [0.0, 1.0] to a character in the ramp.
/// Values outside [0, 1] are clamped.
#[inline]
pub fn ramp(v: f32, chars: &[char]) -> char {
    if chars.is_empty() {
        return ' ';
    }
    let v = v.clamp(0.0, 1.0);
    let idx = (v * (chars.len() - 1) as f32).round() as usize;
    chars[idx.min(chars.len() - 1)]
}

/// Map a value [0.0, 1.0] to an index in the ramp.
#[inline]
pub fn ramp_index(v: f32, len: usize) -> usize {
    if len == 0 {
        return 0;
    }
    let v = v.clamp(0.0, 1.0);
    let idx = (v * (len - 1) as f32).round() as usize;
    idx.min(len - 1)
}

/// Invert a ramp (reverse order).
pub fn invert(ramp: &[char]) -> Vec<char> {
    ramp.iter().rev().copied().collect()
}

/// Named ramp lookup
pub fn by_name(name: &str) -> &'static [char] {
    match name {
        "classic" => RAMP_CLASSIC,
        "donut" => RAMP_DONUT,
        "dots" => RAMP_DOTS,
        "blocks" => RAMP_BLOCKS,
        "extended" => RAMP_EXTENDED,
        "simple" => RAMP_SIMPLE,
        "lines" => RAMP_LINES,
        "matrix" => RAMP_MATRIX,
        "binary" => RAMP_BINARY,
        "hex" => RAMP_HEX,
        _ => RAMP_CLASSIC,
    }
}

/// List of available ramp names
pub const RAMP_NAMES: &[&str] = &[
    "classic", "donut", "dots", "blocks", "extended", "simple", "lines", "matrix", "binary", "hex",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ramp_basic() {
        // 0.0 should give first char
        assert_eq!(ramp(0.0, RAMP_CLASSIC), ' ');
        // 1.0 should give last char
        assert_eq!(ramp(1.0, RAMP_CLASSIC), '@');
        // 0.5 should give middle-ish
        let mid = ramp(0.5, RAMP_CLASSIC);
        assert!(RAMP_CLASSIC.contains(&mid));
    }

    #[test]
    fn test_ramp_clamp() {
        // Values outside [0, 1] should be clamped
        assert_eq!(ramp(-0.5, RAMP_CLASSIC), ' ');
        assert_eq!(ramp(1.5, RAMP_CLASSIC), '@');
    }

    #[test]
    fn test_empty_ramp() {
        assert_eq!(ramp(0.5, &[]), ' ');
    }

    #[test]
    fn test_by_name() {
        assert_eq!(by_name("donut"), RAMP_DONUT);
        assert_eq!(by_name("nonexistent"), RAMP_CLASSIC); // fallback
    }
}
