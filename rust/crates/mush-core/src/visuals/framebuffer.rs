//! Framebuffer abstraction for visual effects.
//!
//! Effects render to a `Framebuffer` which isolates them from terminal details.

/// RGB color with 8-bit components.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0 };
    pub const WHITE: Color = Color { r: 255, g: 255, b: 255 };
    pub const RED: Color = Color { r: 255, g: 0, b: 0 };
    pub const GREEN: Color = Color { r: 0, g: 255, b: 0 };
    pub const BLUE: Color = Color { r: 0, g: 0, b: 255 };
    pub const YELLOW: Color = Color { r: 255, g: 255, b: 0 };
    pub const CYAN: Color = Color { r: 0, g: 255, b: 255 };
    pub const MAGENTA: Color = Color { r: 255, g: 0, b: 255 };

    /// Create color from RGB values.
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    /// Create color from HSV values.
    /// h: 0.0-1.0 (hue), s: 0.0-1.0 (saturation), v: 0.0-1.0 (value)
    pub fn from_hsv(h: f32, s: f32, v: f32) -> Self {
        let h = h.fract();
        let h = if h < 0.0 { h + 1.0 } else { h };
        let s = s.clamp(0.0, 1.0);
        let v = v.clamp(0.0, 1.0);

        let i = (h * 6.0) as i32;
        let f = h * 6.0 - i as f32;
        let p = v * (1.0 - s);
        let q = v * (1.0 - f * s);
        let t = v * (1.0 - (1.0 - f) * s);

        let (r, g, b) = match i % 6 {
            0 => (v, t, p),
            1 => (q, v, p),
            2 => (p, v, t),
            3 => (p, q, v),
            4 => (t, p, v),
            _ => (v, p, q),
        };

        Self {
            r: (r * 255.0) as u8,
            g: (g * 255.0) as u8,
            b: (b * 255.0) as u8,
        }
    }

    /// Linear interpolation between two colors.
    pub fn lerp(a: Color, b: Color, t: f32) -> Self {
        let t = t.clamp(0.0, 1.0);
        Self {
            r: (a.r as f32 + (b.r as f32 - a.r as f32) * t) as u8,
            g: (a.g as f32 + (b.g as f32 - a.g as f32) * t) as u8,
            b: (a.b as f32 + (b.b as f32 - a.b as f32) * t) as u8,
        }
    }

    /// Convert to grayscale value 0.0-1.0.
    pub fn luminance(&self) -> f32 {
        (0.299 * self.r as f32 + 0.587 * self.g as f32 + 0.114 * self.b as f32) / 255.0
    }
}

impl Default for Color {
    fn default() -> Self {
        Self::BLACK
    }
}

/// A single cell in the framebuffer.
#[derive(Clone, Copy, Debug)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::WHITE,
            bg: Color::BLACK,
        }
    }
}

impl Cell {
    pub const fn new(ch: char, fg: Color, bg: Color) -> Self {
        Self { ch, fg, bg }
    }
}

/// Terminal framebuffer for rendering visual effects.
pub struct Framebuffer {
    width: u16,
    height: u16,
    cells: Vec<Cell>,
}

impl Framebuffer {
    /// Create a new framebuffer with the given dimensions.
    pub fn new(width: u16, height: u16) -> Self {
        let size = (width as usize) * (height as usize);
        Self {
            width,
            height,
            cells: vec![Cell::default(); size],
        }
    }

    /// Get framebuffer dimensions.
    pub fn size(&self) -> (u16, u16) {
        (self.width, self.height)
    }

    /// Get width.
    pub fn width(&self) -> u16 {
        self.width
    }

    /// Get height.
    pub fn height(&self) -> u16 {
        self.height
    }

    /// Resize framebuffer, clearing contents.
    pub fn resize(&mut self, width: u16, height: u16) {
        self.width = width;
        self.height = height;
        let size = (width as usize) * (height as usize);
        self.cells.resize(size, Cell::default());
        self.clear();
    }

    /// Clear all cells to default (space, black).
    pub fn clear(&mut self) {
        for cell in &mut self.cells {
            *cell = Cell::default();
        }
    }

    /// Clear all cells with specific background color.
    pub fn clear_with_bg(&mut self, bg: Color) {
        for cell in &mut self.cells {
            cell.ch = ' ';
            cell.fg = Color::WHITE;
            cell.bg = bg;
        }
    }

    /// Get index for coordinates. Returns None if out of bounds.
    fn index(&self, x: u16, y: u16) -> Option<usize> {
        if x < self.width && y < self.height {
            Some((y as usize) * (self.width as usize) + (x as usize))
        } else {
            None
        }
    }

    /// Set a cell's character and colors.
    pub fn set_cell(&mut self, x: u16, y: u16, ch: char, fg: Color, bg: Color) {
        if let Some(idx) = self.index(x, y) {
            self.cells[idx] = Cell { ch, fg, bg };
        }
    }

    /// Set only the character, keeping existing colors. Default fg=white, bg=black if new.
    pub fn set_char(&mut self, x: u16, y: u16, ch: char) {
        if let Some(idx) = self.index(x, y) {
            self.cells[idx].ch = ch;
        }
    }

    /// Set character with foreground color, black background.
    pub fn set_char_fg(&mut self, x: u16, y: u16, ch: char, fg: Color) {
        if let Some(idx) = self.index(x, y) {
            self.cells[idx] = Cell { ch, fg, bg: Color::BLACK };
        }
    }

    /// Get a cell reference.
    pub fn get_cell(&self, x: u16, y: u16) -> Option<&Cell> {
        self.index(x, y).map(|idx| &self.cells[idx])
    }

    /// Get a mutable cell reference.
    pub fn get_cell_mut(&mut self, x: u16, y: u16) -> Option<&mut Cell> {
        self.index(x, y).map(|idx| &mut self.cells[idx])
    }

    /// Fill entire buffer with a character and colors.
    pub fn fill(&mut self, ch: char, fg: Color, bg: Color) {
        let cell = Cell { ch, fg, bg };
        for c in &mut self.cells {
            *c = cell;
        }
    }

    /// Draw a horizontal line.
    pub fn hline(&mut self, x: u16, y: u16, len: u16, ch: char, fg: Color, bg: Color) {
        for dx in 0..len {
            self.set_cell(x.saturating_add(dx), y, ch, fg, bg);
        }
    }

    /// Draw a vertical line.
    pub fn vline(&mut self, x: u16, y: u16, len: u16, ch: char, fg: Color, bg: Color) {
        for dy in 0..len {
            self.set_cell(x, y.saturating_add(dy), ch, fg, bg);
        }
    }

    /// Convert to vector of strings (for compatibility with existing render system).
    /// Each string is one row. Only includes character, ignores color.
    pub fn to_strings(&self) -> Vec<String> {
        let mut result = Vec::with_capacity(self.height as usize);
        for y in 0..self.height {
            let start = (y as usize) * (self.width as usize);
            let end = start + (self.width as usize);
            let s: String = self.cells[start..end].iter().map(|c| c.ch).collect();
            result.push(s);
        }
        result
    }

    /// Convert to vector of ANSI-colored strings with run-length encoding.
    /// Each string is one row with embedded color escape codes.
    /// Colors are only output when they change from the previous cell (RLE).
    pub fn to_colored_strings(&self) -> Vec<String> {
        let mut result = Vec::with_capacity(self.height as usize);
        for y in 0..self.height {
            let start = (y as usize) * (self.width as usize);
            let end = start + (self.width as usize);
            let row = &self.cells[start..end];
            
            let mut s = String::with_capacity(self.width as usize * 6); // smaller estimate with RLE
            let mut last_color: Option<Color> = None;
            
            for cell in row {
                // Only output color escape if color changed
                if last_color != Some(cell.fg) {
                    s.push_str(&format!("\x1b[38;2;{};{};{}m", cell.fg.r, cell.fg.g, cell.fg.b));
                    last_color = Some(cell.fg);
                }
                s.push(cell.ch);
            }
            s.push_str("\x1b[0m"); // reset at end of line
            result.push(s);
        }
        result
    }

    /// Get raw cell slice for efficient iteration.
    pub fn cells(&self) -> &[Cell] {
        &self.cells
    }

    /// Get row slice.
    pub fn row(&self, y: u16) -> &[Cell] {
        if y >= self.height {
            return &[];
        }
        let start = (y as usize) * (self.width as usize);
        let end = start + (self.width as usize);
        &self.cells[start..end]
    }

    /// Apply a brightness transformation to all cells.
    /// The transform function receives (brightness, x, y, width, height) and returns new brightness.
    /// Brightness is derived from foreground color luminance and scaled back to color.
    pub fn apply_brightness_fx<F>(&mut self, transform: F)
    where
        F: Fn(f32, usize, usize, usize, usize) -> f32,
    {
        let w = self.width as usize;
        let h = self.height as usize;
        
        for y in 0..self.height {
            for x in 0..self.width {
                if let Some(idx) = self.index(x, y) {
                    let cell = &mut self.cells[idx];
                    
                    // Get current brightness from fg color luminance
                    let old_lum = cell.fg.luminance();
                    if old_lum < 0.01 {
                        continue; // Skip very dark cells
                    }
                    
                    // Apply transformation
                    let new_lum = transform(old_lum, x as usize, y as usize, w, h).clamp(0.0, 1.0);
                    
                    // Scale color by brightness ratio
                    let ratio = new_lum / old_lum.max(0.01);
                    cell.fg = Color {
                        r: (cell.fg.r as f32 * ratio).min(255.0) as u8,
                        g: (cell.fg.g as f32 * ratio).min(255.0) as u8,
                        b: (cell.fg.b as f32 * ratio).min(255.0) as u8,
                    };
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_framebuffer_basic() {
        let mut fb = Framebuffer::new(10, 5);
        assert_eq!(fb.size(), (10, 5));
        
        fb.set_cell(0, 0, 'A', Color::WHITE, Color::BLACK);
        let cell = fb.get_cell(0, 0).unwrap();
        assert_eq!(cell.ch, 'A');
        
        // Out of bounds should be safe
        fb.set_cell(100, 100, 'X', Color::WHITE, Color::BLACK);
        assert!(fb.get_cell(100, 100).is_none());
    }

    #[test]
    fn test_color_hsv() {
        // Red at 0 hue
        let red = Color::from_hsv(0.0, 1.0, 1.0);
        assert_eq!(red.r, 255);
        assert_eq!(red.g, 0);
        assert_eq!(red.b, 0);
        
        // Green at 1/3 hue
        let green = Color::from_hsv(1.0 / 3.0, 1.0, 1.0);
        assert_eq!(green.r, 0);
        assert_eq!(green.g, 255);
        assert!(green.b <= 1); // Might be 0 or 1 due to floating point
        
        // White at zero saturation
        let white = Color::from_hsv(0.5, 0.0, 1.0);
        assert_eq!(white.r, 255);
        assert_eq!(white.g, 255);
        assert_eq!(white.b, 255);
    }

    #[test]
    fn test_to_strings() {
        let mut fb = Framebuffer::new(3, 2);
        fb.set_char(0, 0, 'A');
        fb.set_char(1, 0, 'B');
        fb.set_char(2, 0, 'C');
        fb.set_char(0, 1, 'D');
        fb.set_char(1, 1, 'E');
        fb.set_char(2, 1, 'F');
        
        let strings = fb.to_strings();
        // Default char is space, so unfilled cells are spaces
        // Actually we set all chars, so:
        assert_eq!(strings.len(), 2);
    }
}
