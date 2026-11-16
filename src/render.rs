//! Software rendering: font rasterisation + framebuffer drawing.
//!
//! Text is rasterised with the `fontdue` crate (bundled DejaVu fonts in
//! `assets/fonts/`). All drawing targets a plain `&mut [u32]` pixel buffer
//! in `0xRRGGBB` order, which is exactly what `softbuffer` expects —
//! so the renderer is testable without a window.

use crate::css::Color;
use crate::layout::LayoutBox;
use fontdue::{Font, FontSettings};

/// Bundled fonts (DejaVu Sans — permissive licence, ships with most Linuxes).
const FONT_REGULAR: &[u8] = include_bytes!("../assets/fonts/DejaVuSans.ttf");
const FONT_BOLD: &[u8] = include_bytes!("../assets/fonts/DejaVuSans-Bold.ttf");
/// Synthetic oblique slant: horizontal shift per pixel of glyph height.
/// DejaVu Sans ships no true italic in the bundled set, so italics are
/// produced by shearing the regular glyph bitmap (same trick as many
/// lightweight text stacks).
const OBLIQUE_SHEAR: f32 = 0.21;

/// A set of rasterisable fonts with text-measurement helpers shared by
/// the layout engine (wrapping) and the renderer (drawing).
pub struct FontSet {
    regular: Font,
    bold: Font,
}

impl FontSet {
    pub fn load() -> Self {
        let settings = FontSettings::default();
        Self {
            regular: Font::from_bytes(FONT_REGULAR, settings).expect("regular font"),
            bold: Font::from_bytes(FONT_BOLD, settings).expect("bold font"),
        }
    }

    /// Italic is synthesised by shearing the upright face, so the italic
    /// flag does not select a different font file.
    fn font(&self, bold: bool, _italic: bool) -> &Font {
        if bold {
            &self.bold
        } else {
            &self.regular
        }
    }

    /// Advance width of `text` at `size` px.
    pub fn width(&self, text: &str, size: f32, bold: bool, italic: bool) -> f32 {
        let font = self.font(bold, italic);
        text.chars()
            .map(|c| {
                font.horizontal_line_metrics(size)
                    .map(|_| font.metrics(c, size).advance_width)
                    .unwrap_or(size * 0.5)
            })
            .sum()
    }

    /// Line height in px for a font size (ascent + descent + leading).
    pub fn line_height(&self, size: f32) -> f32 {
        size * 1.25
    }

    /// Rasterise one glyph. Returns (bitmap, width, height, xmin, ymin).
    pub fn rasterize(
        &self,
        c: char,
        size: f32,
        bold: bool,
        italic: bool,
    ) -> (Vec<u8>, usize, usize, i32, i32) {
        let font = self.font(bold, italic);
        let (metrics, bitmap) = font.rasterize(c, size);
        (
            bitmap,
            metrics.width,
            metrics.height,
            metrics.xmin,
            metrics.ymin,
        )
    }

    /// Advance width of a single glyph (for pen positioning).
    pub fn advance(&self, c: char, size: f32, bold: bool, italic: bool) -> f32 {
        self.font(bold, italic).metrics(c, size).advance_width
    }
}

/// Pack an RGB colour into a softbuffer pixel (`0xRRGGBB`).
pub fn pack(c: Color) -> u32 {
    ((c.r as u32) << 16) | ((c.g as u32) << 8) | (c.b as u32)
}

/// Alpha-blend `fg` over `bg` (`alpha` 0..=255).
pub fn blend(fg: Color, bg: Color, alpha: u8) -> Color {
    let a = alpha as u32;
    let inv = 255 - a;
    Color::rgb(
        ((fg.r as u32 * a + bg.r as u32 * inv) / 255) as u8,
        ((fg.g as u32 * a + bg.g as u32 * inv) / 255) as u8,
        ((fg.b as u32 * a + bg.b as u32 * inv) / 255) as u8,
    )
}

/// A minimal framebuffer over a `&mut [u32]` slice.
pub struct Framebuffer<'a> {
    pub pixels: &'a mut [u32],
    pub width: usize,
    pub height: usize,
}

impl<'a> Framebuffer<'a> {
    pub fn new(pixels: &'a mut [u32], width: usize, height: usize) -> Self {
        Self {
            pixels,
            width,
            height,
        }
    }

    pub fn fill(&mut self, color: Color) {
        self.pixels.fill(pack(color));
    }

    pub fn set_pixel(&mut self, x: i32, y: i32, color: Color) {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height {
            return;
        }
        self.pixels[y as usize * self.width + x as usize] = pack(color);
    }

    /// Filled rectangle (clipped to the framebuffer).
    pub fn fill_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        let x0 = x.floor() as i32;
        let y0 = y.floor() as i32;
        let x1 = (x + w).ceil() as i32;
        let y1 = (y + h).ceil() as i32;
        let xa = x0.max(0);
        let ya = y0.max(0);
        let xb = x1.min(self.width as i32);
        let yb = y1.min(self.height as i32);
        if xa >= xb || ya >= yb {
            return;
        }
        let packed = pack(color);
        for row in ya..yb {
            let start = row as usize * self.width + xa as usize;
            let end = row as usize * self.width + xb as usize;
            self.pixels[start..end].fill(packed);
        }
    }

    /// 1px outline rectangle.
    pub fn stroke_rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        self.fill_rect(x, y, w, 1.0, color);
        self.fill_rect(x, y + h - 1.0, w, 1.0, color);
        self.fill_rect(x, y, 1.0, h, color);
        self.fill_rect(x + w - 1.0, y, 1.0, h, color);
    }

    /// Draw text with its top-left at (`x`, `y`) (baseline handled internally).
    /// Returns the advance width so callers can chain runs.
    #[allow(clippy::too_many_arguments)]
    pub fn draw_text(
        &mut self,
        x: f32,
        y: f32,
        text: &str,
        size: f32,
        color: Color,
        bold: bool,
        italic: bool,
        fonts: &FontSet,
    ) -> f32 {
        let mut pen_x = x;
        let baseline = y + size; // simple baseline: top + size
        for c in text.chars() {
            if c == '\n' {
                continue;
            }
            let (bitmap, gw, gh, xmin, ymin) = fonts.rasterize(c, size, bold, italic);
            let gx = (pen_x + xmin as f32).round() as i32;
            let gy = (baseline - ymin as f32 - gh as f32).round() as i32;
            for row in 0..gh {
                for col in 0..gw {
                    let alpha = bitmap[row * gw + col];
                    if alpha == 0 {
                        continue;
                    }
                    // Slant the bitmap for synthetic italics (top rows shift
                    // further right than bottom rows).
                    let shear = if italic {
                        (((gh - 1 - row) as f32) * OBLIQUE_SHEAR).round() as i32
                    } else {
                        0
                    };
                    let px = gx + col as i32 + shear;
                    let py = gy + row as i32;
                    if px < 0 || py < 0 || px as usize >= self.width || py as usize >= self.height {
                        continue;
                    }
                    let idx = py as usize * self.width + px as usize;
                    let bg_packed = self.pixels[idx];
                    let bg = Color::rgb(
                        ((bg_packed >> 16) & 0xFF) as u8,
                        ((bg_packed >> 8) & 0xFF) as u8,
                        (bg_packed & 0xFF) as u8,
                    );
                    self.pixels[idx] = pack(blend(color, bg, alpha));
                }
            }
            pen_x += fonts.advance(c, size, bold, italic);
        }
        pen_x - x
    }

    /// Centred text inside a rect (for buttons / labels).
    pub fn draw_text_centered(
        &mut self,
        rect: (f32, f32, f32, f32),
        text: &str,
        size: f32,
        color: Color,
        bold: bool,
        fonts: &FontSet,
    ) {
        let (x, y, w, h) = rect;
        let tw = fonts.width(text, size, bold, false);
        let tx = x + (w - tw) / 2.0;
        let ty = y + (h - size) / 2.0;
        self.draw_text(tx, ty, text, size, color, bold, false, fonts);
    }
}

/// Render a laid-out page into the framebuffer.
/// `scroll_y` shifts content up; `viewport` clips drawing.
pub fn render_page(
    fb: &mut Framebuffer,
    root: &LayoutBox,
    scroll_y: f32,
    viewport: (f32, f32, f32, f32),
    fonts: &FontSet,
) {
    let (vx, vy, vw, vh) = viewport;
    // Page background.
    fb.fill_rect(vx, vy, vw, vh, Color::rgb(255, 255, 255));
    render_box(fb, root, (vx, vy - scroll_y), viewport, fonts);
}

/// `origin` translates page coordinates (absolute, from `layout`) into
/// framebuffer coordinates: the viewport sits at `(vx, vy)` and scrolls up
/// by `scroll_y`.
fn render_box(
    fb: &mut Framebuffer,
    b: &LayoutBox,
    origin: (f32, f32),
    viewport: (f32, f32, f32, f32),
    fonts: &FontSet,
) {
    let (vx, vy, vw, vh) = viewport;
    let (ox, oy) = origin;
    let x = b.x + ox;
    let y = b.y + oy;
    // Clip: skip boxes fully outside the viewport.
    if y + b.height < vy || y > vy + vh || x + b.width < vx || x > vx + vw {
        // Still need to visit children? No — if the parent is off-screen
        // vertically, children are too (layout is top-down). Safe to skip.
        if y + b.height < vy || y > vy + vh {
            return;
        }
    }

    if let Some(bg) = b.style.background {
        fb.fill_rect(x, y, b.width, b.height, bg);
    }

    match &b.kind {
        crate::layout::BoxKind::Text(text) => {
            let size = b.style.font_size;
            let bold = b.style.font_weight >= 600;
            fb.draw_text(x, y, text, size, b.style.color, bold, b.style.italic, fonts);
            if b.style.underline {
                let ty = y + size + 2.0;
                fb.fill_rect(x, ty, b.width, 1.0, b.style.color);
            }
        }
        crate::layout::BoxKind::Image { alt, .. } => {
            // Placeholder: grey box with alt text (real image decoding is
            // on the roadmap — see README).
            fb.fill_rect(x, y, b.width, b.height, Color::rgb(220, 220, 220));
            fb.stroke_rect(x, y, b.width, b.height, Color::rgb(160, 160, 160));
            if !alt.is_empty() {
                fb.draw_text(
                    x + 4.0,
                    y + 4.0,
                    alt,
                    12.0,
                    Color::rgb(80, 80, 80),
                    false,
                    false,
                    fonts,
                );
            }
        }
        crate::layout::BoxKind::Br => {}
        // List items get a bullet in their (indented) left margin.
        crate::layout::BoxKind::Block { tag } if tag == "li" => {
            fb.draw_text(
                x - 18.0,
                y,
                "•",
                b.style.font_size,
                b.style.color,
                false,
                false,
                fonts,
            );
        }
        crate::layout::BoxKind::Block { .. } | crate::layout::BoxKind::Inline { .. } => {}
    }

    for child in &b.children {
        render_box(fb, child, origin, viewport, fonts);
    }
}

/// Draw a simple message page (start page / error page) as a layout-free
/// fallback: centred text lines.
pub fn render_message(
    fb: &mut Framebuffer,
    viewport: (f32, f32, f32, f32),
    title: &str,
    lines: &[&str],
    fonts: &FontSet,
) {
    let (vx, vy, vw, vh) = viewport;
    fb.fill_rect(vx, vy, vw, vh, Color::rgb(255, 255, 255));
    let cx = vx + vw / 2.0;
    let mut y = vy + vh / 2.0 - 80.0;
    let tw = fonts.width(title, 24.0, true, false);
    fb.draw_text(
        cx - tw / 2.0,
        y,
        title,
        24.0,
        Color::rgb(30, 30, 30),
        true,
        false,
        fonts,
    );
    y += 48.0;
    for line in lines {
        let w = fonts.width(line, 14.0, false, false);
        fb.draw_text(
            cx - w / 2.0,
            y,
            line,
            14.0,
            Color::rgb(90, 90, 90),
            false,
            false,
            fonts,
        );
        y += 22.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pack_and_blend() {
        assert_eq!(pack(Color::rgb(0x12, 0x34, 0x56)), 0x123456);
        assert_eq!(
            blend(Color::rgb(255, 0, 0), Color::rgb(0, 0, 0), 255),
            Color::rgb(255, 0, 0)
        );
        assert_eq!(
            blend(Color::rgb(255, 0, 0), Color::rgb(0, 0, 0), 0),
            Color::rgb(0, 0, 0)
        );
    }

    #[test]
    fn framebuffer_clips_and_fills() {
        let mut px = vec![0u32; 10 * 10];
        {
            let mut fb = Framebuffer::new(&mut px, 10, 10);
            fb.fill_rect(-5.0, -5.0, 10.0, 10.0, Color::rgb(255, 0, 0));
        }
        assert_eq!(px[0], 0xFF0000);
        assert_eq!(px[9 * 10 + 9], 0x000000);
    }
}
