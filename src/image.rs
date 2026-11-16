//! Image decoding: PNG/JPEG bytes → `0xRRGGBB` pixels for the framebuffer.
//!
//! Decoding happens on the loader thread (`net::fetch_page`), so the UI
//! thread only ever touches ready-to-blit pixel buffers. The engine has no
//! per-pixel alpha compositing yet, so transparent pixels are flattened
//! onto white when decoding.

use std::collections::HashMap;

/// A decoded image: row-major `0xRRGGBB` pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}

impl DecodedImage {
    /// Decode PNG or JPEG data (format sniffed from the bytes).
    /// Returns `None` for anything we cannot decode — callers fall back to
    /// the placeholder box, so a broken image never breaks a page.
    pub fn decode(bytes: &[u8]) -> Option<Self> {
        let dynamic = image::load_from_memory(bytes).ok()?;
        let rgba = dynamic.to_rgba8();
        let (width, height) = rgba.dimensions();
        if width == 0 || height == 0 {
            return None;
        }
        let mut pixels = Vec::with_capacity((width * height) as usize);
        for p in rgba.pixels() {
            let [r, g, b, a] = p.0;
            if a == 255 {
                pixels.push(((r as u32) << 16) | ((g as u32) << 8) | b as u32);
            } else {
                // Flatten onto white.
                let over = |c: u8| -> u32 { (c as u32 * a as u32 + 255 * (255 - a as u32)) / 255 };
                pixels.push((over(r) << 16) | (over(g) << 8) | over(b));
            }
        }
        Some(Self {
            width: width as usize,
            height: height as usize,
            pixels,
        })
    }

    /// Pixel at (`x`, `y`), clamped into the image.
    pub fn pixel(&self, x: usize, y: usize) -> u32 {
        let x = x.min(self.width.saturating_sub(1));
        let y = y.min(self.height.saturating_sub(1));
        self.pixels[y * self.width + x]
    }

    /// Intrinsic size in CSS pixels (1 image pixel = 1 CSS pixel).
    pub fn size(&self) -> (f32, f32) {
        (self.width as f32, self.height as f32)
    }
}

/// Images for one page, keyed by absolute URL.
///
/// `src` attributes are relative to the page URL, so the store keeps that
/// base and resolves on lookup — the layout engine can then stay ignorant
/// of URLs entirely.
#[derive(Debug, Clone, Default)]
pub struct ImageStore {
    base_url: String,
    map: HashMap<String, DecodedImage>,
}

impl ImageStore {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            map: HashMap::new(),
        }
    }

    pub fn set_base_url(&mut self, url: impl Into<String>) {
        self.base_url = url.into();
    }

    pub fn insert(&mut self, url: String, image: DecodedImage) {
        self.map.insert(url, image);
    }

    /// Look up an image by the raw `src` used in the HTML.
    pub fn get(&self, src: &str) -> Option<&DecodedImage> {
        let key = crate::net::resolve_url(&self.base_url, src)?;
        self.map
            .get(&key)
            .or_else(|| self.map.get(src))
            .or_else(|| {
                // Percent-encoding differences between what we fetched and
                // what the page wrote: fall back to a suffix match.
                self.map
                    .iter()
                    .find(|(k, _)| k.ends_with(src))
                    .map(|(_, v)| v)
            })
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 4x4 test card: red | green on the top row, blue | white below.
    const CARD: &[u8] = include_bytes!("../assets/tests/card.png");

    #[test]
    fn decodes_png_and_reports_size() {
        let img = DecodedImage::decode(CARD).expect("png decodes");
        assert_eq!((img.width, img.height), (4, 4));
        assert_eq!(img.pixels.len(), 16);
    }

    #[test]
    fn pixels_are_rgb_quadrants() {
        let img = DecodedImage::decode(CARD).unwrap();
        assert_eq!(img.pixel(0, 0), 0xFF0000); // red
        assert_eq!(img.pixel(3, 0), 0x008000); // green
        assert_eq!(img.pixel(0, 3), 0x0000FF); // blue
        assert_eq!(img.pixel(3, 3), 0xFFFFFF);
    }

    #[test]
    fn garbage_decodes_to_none() {
        assert!(DecodedImage::decode(b"not an image").is_none());
        assert!(DecodedImage::decode(&[]).is_none());
    }

    #[test]
    fn store_resolves_relative_src() {
        let mut store = ImageStore::new("https://example.com/blog/post.html");
        store.insert(
            "https://example.com/blog/pic.png".to_string(),
            DecodedImage::decode(CARD).unwrap(),
        );
        assert!(store.get("pic.png").is_some());
        assert!(store.get("./pic.png").is_some());
        assert!(store.get("missing.png").is_none());
    }
}
