//! Headless smoke test: parse -> style -> layout -> rasterise -> PNG,
//! with no window or GPU involved.
use rust_browser::css::ua_stylesheet;
use rust_browser::layout::layout_document;
use rust_browser::render::{render_page, FontSet, Framebuffer};

const SAMPLE: &str = r#"<!doctype html>
<html><head><title>Smoke</title></head>
<body>
<h1>Rust Browser</h1>
<p>This page was parsed by <b>our own</b> HTML parser, styled by our own
CSS engine and rasterised with <i>fontdue</i>. No browser engine involved.</p>
<h2>Links</h2>
<p>Visit <a href="https://example.com">example.com</a> to see the hit-testing
work — hover links to see the target in the status bar.</p>
<ul><li>tabs</li><li>history</li><li>word wrap</li></ul>
</body></html>"#;

fn main() {
    let width: usize = 760;
    let height: usize = 520;
    let doc = rust_browser::html::parse(SAMPLE);
    let sheet = ua_stylesheet();
    let fonts = FontSet::load();
    let root = layout_document(&doc, &sheet, width as f32, &fonts);

    let mut pixels = vec![0u32; width * height];
    {
        let mut fb = Framebuffer::new(&mut pixels, width, height);
        render_page(
            &mut fb,
            &root,
            0.0,
            (0.0, 0.0, width as f32, height as f32),
            &fonts,
        );
    }
    write_png(
        "/home/user/rust-browser/target/smoke.png",
        &pixels,
        width,
        height,
    );
    println!("title from <title>: {:?}", doc.title);
    println!("wrote target/smoke.png ({}x{})", width, height);
}

fn write_png(path: &str, pixels: &[u32], w: usize, h: usize) {
    // Minimal PNG writer (truecolour, no compression): keeps the example
    // dependency-free.
    fn crc32(data: &[u8]) -> u32 {
        let mut table = [0u32; 256];
        for i in 0..256u32 {
            let mut c = i;
            for _ in 0..8 {
                c = if c & 1 != 0 {
                    0xEDB8_8320 ^ (c >> 1)
                } else {
                    c >> 1
                };
            }
            table[i as usize] = c;
        }
        let mut crc = 0xFFFF_FFFFu32;
        for &b in data {
            crc = table[((crc ^ b as u32) & 0xFF) as usize] ^ (crc >> 8);
        }
        crc ^ 0xFFFF_FFFF
    }
    fn chunk(out: &mut Vec<u8>, kind: &[u8], data: &[u8]) {
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        let mut body = kind.to_vec();
        body.extend_from_slice(data);
        out.extend_from_slice(&body);
        out.extend_from_slice(&crc32(&body).to_be_bytes());
    }
    let mut raw = Vec::with_capacity(h * (w * 3 + 1));
    for y in 0..h {
        raw.push(0);
        for x in 0..w {
            let p = pixels[y * w + x];
            raw.push((p >> 16) as u8);
            raw.push((p >> 8) as u8);
            raw.push(p as u8);
        }
    }
    // zlib stream with stored (uncompressed) deflate blocks.
    let mut z = vec![0x78, 0x01];
    let mut adler_a: u32 = 1;
    let mut adler_b: u32 = 0;
    for &b in &raw {
        adler_a = (adler_a + b as u32) % 65521;
        adler_b = (adler_b + adler_a) % 65521;
    }
    for (i, block) in raw.chunks(65535).enumerate() {
        let last = (i + 1) * 65535 >= raw.len();
        z.push(if last { 1 } else { 0 });
        z.extend_from_slice(&(block.len() as u16).to_le_bytes());
        z.extend_from_slice(&(!(block.len() as u16)).to_le_bytes());
        z.extend_from_slice(block);
    }
    z.extend_from_slice(&((adler_b << 16) | adler_a).to_be_bytes());

    let mut png = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    let mut ihdr = Vec::new();
    ihdr.extend_from_slice(&(w as u32).to_be_bytes());
    ihdr.extend_from_slice(&(h as u32).to_be_bytes());
    ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
    chunk(&mut png, b"IHDR", &ihdr);
    chunk(&mut png, b"IDAT", &z);
    chunk(&mut png, b"IEND", &[]);
    std::fs::write(path, png).expect("write png");
}
