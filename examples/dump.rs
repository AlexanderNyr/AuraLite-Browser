use rust_browser::layout::{layout_document, LayoutBox};

fn dump(b: &LayoutBox, depth: usize) {
    println!(
        "{}{:?} x={:.1} y={:.1} w={:.1} h={:.1} link={:?}",
        "  ".repeat(depth),
        b.kind,
        b.x,
        b.y,
        b.width,
        b.height,
        b.link
    );
    for c in &b.children {
        dump(c, depth + 1);
    }
}

fn main() {
    let path = std::env::args().nth(1);
    let html = match path {
        Some(p) => std::fs::read_to_string(p).unwrap(),
        None => r#"<body><h1>Rust Browser</h1><p>This page was parsed by <b>our own</b> parser.</p><h2>Links</h2><p>Visit <a href="https://example.com">example.com</a> now.</p><ul><li>tabs</li><li>history</li></ul></body>"#.to_string(),
    };
    let doc = rust_browser::html::parse(&html);
    let sheet = rust_browser::css::ua_stylesheet();
    let fonts = rust_browser::render::FontSet::load();
    let root = layout_document(&doc, &sheet, 760.0, &fonts);
    dump(&root, 0);
}
