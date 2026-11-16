//! End-to-end test of the engine with no window and no network:
//! HTML string -> DOM -> CSS -> layout -> rasterised pixels.

use rust_browser::css::ua_stylesheet;
use rust_browser::layout::{hit_test, layout_document};
use rust_browser::render::{render_page, FontSet, Framebuffer};

const PAGE: &str = r#"<!doctype html>
<html><head><title>Pipeline</title></head>
<body>
  <h1>Title</h1>
  <p>Some text with a <a href="/relative">link</a> inside.</p>
  <ul><li>one</li><li>two</li></ul>
</body></html>"#;

fn render(width: usize, height: usize) -> (Vec<u32>, rust_browser::layout::LayoutBox) {
    let doc = rust_browser::html::parse(PAGE);
    let sheet = ua_stylesheet();
    let fonts = FontSet::load();
    let images = rust_browser::image::ImageStore::default();
    let root = layout_document(&doc, &sheet, width as f32, &fonts, &images);
    let mut pixels = vec![0u32; width * height];
    let mut fb = Framebuffer::new(&mut pixels, width, height);
    render_page(
        &mut fb,
        &root,
        0.0,
        (0.0, 0.0, width as f32, height as f32),
        &fonts,
        &images,
    );
    (pixels, root)
}

#[test]
fn renders_non_blank_page() {
    let (pixels, root) = render(640, 480);
    assert!(root.height > 0.0);
    let inked = pixels.iter().filter(|&&p| p != 0x00FF_FFFF).count();
    assert!(inked > 500, "expected text pixels, got {inked}");
}

#[test]
fn document_title_is_extracted() {
    let doc = rust_browser::html::parse(PAGE);
    assert_eq!(doc.title, "Pipeline");
}

#[test]
fn link_is_clickable_after_layout() {
    let (_pixels, root) = render(640, 480);
    // Scan the page for the point where the link box starts and hit-test it.
    fn find(b: &rust_browser::layout::LayoutBox) -> Option<(f32, f32)> {
        if b.link.is_some() {
            return Some((b.x + 1.0, b.y + b.height / 2.0));
        }
        b.children.iter().find_map(find)
    }
    let (x, y) = find(&root).expect("link present in layout");
    assert_eq!(
        hit_test(&root, x, y).and_then(|h| h.link).as_deref(),
        Some("/relative")
    );
}

#[test]
fn scrolling_moves_content_up() {
    let doc = rust_browser::html::parse(&format!("<body>{}</body>", "<p>line</p>".repeat(80)));
    let sheet = ua_stylesheet();
    let fonts = FontSet::load();
    let images_ctx = rust_browser::image::ImageStore::default();
    let root = layout_document(&doc, &sheet, 300.0, &fonts, &images_ctx);
    assert!(root.height > 480.0, "page should overflow the viewport");
    let mut top = vec![0u32; 300 * 480];
    let mut scrolled = vec![0u32; 300 * 480];
    Framebuffer::new(&mut top, 300, 480).fill_rect(
        0.0,
        0.0,
        300.0,
        480.0,
        rust_browser::css::Color::rgb(255, 255, 255),
    );
    Framebuffer::new(&mut scrolled, 300, 480).fill_rect(
        0.0,
        0.0,
        300.0,
        480.0,
        rust_browser::css::Color::rgb(255, 255, 255),
    );
    render_page(
        &mut Framebuffer::new(&mut top, 300, 480),
        &root,
        0.0,
        (0.0, 0.0, 300.0, 480.0),
        &fonts,
        &images_ctx,
    );
    render_page(
        &mut Framebuffer::new(&mut scrolled, 300, 480),
        &root,
        200.0,
        (0.0, 0.0, 300.0, 480.0),
        &fonts,
        &images_ctx,
    );
    assert_ne!(top, scrolled, "scrolling should change what is visible");
}

/// Regression: a scrolled page must not paint over the browser chrome.
/// The viewport is a rectangle inside the framebuffer; everything outside
/// it has to keep whatever the chrome drew there.
#[test]
fn page_never_paints_outside_the_viewport() {
    use rust_browser::css::Stylesheet;

    let html = format!("<body>{}</body>", "<p>line of text</p>".repeat(40));
    let doc = rust_browser::html::parse(&html);
    let mut sheet = ua_stylesheet();
    sheet.extend(Stylesheet::parse("body { background: #ff0000 }"));

    let fonts = FontSet::load();
    let images = rust_browser::image::ImageStore::default();
    let root = layout_document(&doc, &sheet, 400.0, &fonts, &images);

    const W: usize = 400;
    const H: usize = 500;
    const CHROME: u32 = 0x0000FF;
    let viewport = (0.0, 100.0, 400.0, 300.0);

    let mut pixels = vec![CHROME; W * H];
    let mut fb = Framebuffer::new(&mut pixels, W, H);
    render_page(&mut fb, &root, 250.0, viewport, &fonts, &images);

    // Rows above and below the viewport must be untouched.
    for y in 0..100 {
        for x in 0..W {
            assert_eq!(pixels[y * W + x], CHROME, "chrome row {y} was painted over");
        }
    }
    for y in 400..H {
        for x in 0..W {
            assert_eq!(
                pixels[y * W + x],
                CHROME,
                "status-bar row {y} was painted over"
            );
        }
    }
    // And the viewport itself must have content (red or text pixels).
    let painted = (100..400)
        .flat_map(|y| (0..W).map(move |x| (x, y)))
        .filter(|&(x, y)| pixels[y * W + x] != CHROME)
        .count();
    assert!(painted > 1000, "viewport looks empty: {painted} pixels");
}
