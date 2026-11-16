//! Stage-by-stage timing for a real page: fetch -> parse -> style -> layout.
use std::time::Instant;

fn main() {
    let url = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "https://en.wikipedia.org/wiki/Rust_(programming_language)".into());
    let t = Instant::now();
    let page = match rust_browser::net::fetch_page(&url) {
        Ok(p) => p,
        Err(e) => {
            println!("fetch failed: {e}");
            return;
        }
    };
    println!(
        "fetch:      {:?} ({} html bytes, {} sheets)",
        t.elapsed(),
        page.html.len(),
        page.linked_css.len()
    );

    let t = Instant::now();
    let doc = rust_browser::html::parse(&page.html);
    println!("parse:      {:?}", t.elapsed());

    let t = Instant::now();
    let mut sheet = rust_browser::css::ua_stylesheet();
    for css in &page.linked_css {
        sheet.extend(rust_browser::css::Stylesheet::parse(css));
    }
    for css in doc.style_texts() {
        sheet.extend(rust_browser::css::Stylesheet::parse(&css));
    }
    println!("css:        {:?}", t.elapsed());

    let fonts = rust_browser::render::FontSet::load();
    let images = rust_browser::image::ImageStore::default();
    let t = Instant::now();
    let root = rust_browser::layout::layout_document(&doc, &sheet, 1100.0, &fonts, &images);
    println!("layout:     {:?} (height {:.0})", t.elapsed(), root.height);

    let mut pixels = vec![0u32; 1100 * 700];
    let t = Instant::now();
    {
        let mut fb = rust_browser::render::Framebuffer::new(&mut pixels, 1100, 700);
        rust_browser::render::render_page(
            &mut fb,
            &root,
            0.0,
            (0.0, 0.0, 1100.0, 700.0),
            &fonts,
            &images,
        );
    }
    println!("render:     {:?}", t.elapsed());
}
