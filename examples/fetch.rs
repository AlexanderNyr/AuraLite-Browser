//! Fetch a URL through the engine's own loader and report timings.
use std::time::Instant;

fn main() {
    let url = std::env::args().nth(1).expect("usage: fetch <url>");
    let t = Instant::now();
    match rust_browser::net::fetch_page(&url) {
        Ok(page) => {
            println!(
                "ok in {:?}: {} bytes html, {} stylesheets, {} image urls",
                t.elapsed(),
                page.html.len(),
                page.linked_css.len(),
                page.image_urls.len()
            );
        }
        Err(e) => println!("error in {:?}: {e}", t.elapsed()),
    }
}
