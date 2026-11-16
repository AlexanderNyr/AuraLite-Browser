//! Networking: HTTP/HTTPS fetching with `ureq` (rustls for TLS).
//!
//! Fetching runs on a background thread (see `browser::Tab::navigate`)
//! so the UI thread never blocks on the network.

use crate::image::DecodedImage;
use std::io::Read;
use std::time::Duration;

/// Hard cap per image so one huge asset cannot exhaust memory.
const MAX_IMAGE_BYTES: u64 = 12 * 1024 * 1024;
/// Cap on how many images one page load will fetch.
const MAX_IMAGES_PER_PAGE: usize = 48;
/// Images are downloaded by this many threads at once.
const IMAGE_WORKERS: usize = 6;
/// Images get a shorter deadline than the document itself.
const IMAGE_TIMEOUT: Duration = Duration::from_secs(8);
use url::Url;

/// A fully fetched page: HTML plus any linked stylesheets.
#[derive(Debug, Clone)]
pub struct FetchedPage {
    pub final_url: String,
    pub html: String,
    /// Contents of `<link rel="stylesheet">` files, in document order.
    pub linked_css: Vec<String>,
    /// Absolute URLs of images on the page, de-duplicated, for the
    /// background image loader.
    pub image_urls: Vec<String>,
}

/// Fetch `url` (following redirects) and its linked stylesheets.
///
/// Images are *not* fetched here: the returned [`FetchedPage::image_urls`]
/// list is handed to [`fetch_images`] on a background thread so the page can
/// be laid out and shown immediately.
pub fn fetch_page(url: &str) -> Result<FetchedPage, String> {
    let (final_url, html) = fetch_text(url)?;
    let doc = crate::html::parse(&html);
    let mut linked_css = Vec::new();
    for href in doc.stylesheet_hrefs() {
        if let Some(css_url) = resolve_url(&final_url, &href) {
            // Best-effort: a broken stylesheet must not break the page.
            if let Ok((_, css)) = fetch_text(&css_url) {
                linked_css.push(css);
            }
        }
    }
    let image_urls = image_urls_for(&final_url, doc.image_srcs());
    Ok(FetchedPage {
        final_url,
        html,
        linked_css,
        image_urls,
    })
}

/// Resolve, de-duplicate and cap the page's image `src` values.
fn image_urls_for(base_url: &str, srcs: Vec<String>) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    srcs.into_iter()
        .filter(|s| !s.trim().is_empty())
        .filter_map(|s| resolve_url(base_url, &s))
        .filter(|u| seen.insert(u.clone()))
        .take(MAX_IMAGES_PER_PAGE)
        .collect()
}

/// Download and decode a page's images, several at a time.
///
/// Sequential downloads made image-heavy pages crawl, so the URLs are pulled
/// by a small pool of workers and collected through a mutex. Duplicate `src`
/// values (very common: logos, sprites) are fetched once.
pub fn fetch_images(urls: Vec<String>) -> Vec<(String, DecodedImage)> {
    if urls.is_empty() {
        return Vec::new();
    }

    let queue = std::sync::Mutex::new(urls.into_iter());
    let results: std::sync::Mutex<Vec<(String, DecodedImage)>> = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..IMAGE_WORKERS.min(8) {
            scope.spawn(|| loop {
                let next = queue.lock().ok().and_then(|mut q| q.next());
                let Some(url) = next else {
                    break;
                };
                if let Ok((_, bytes)) = fetch_bytes_with_timeout(&url, IMAGE_TIMEOUT) {
                    if let Some(image) = DecodedImage::decode(&bytes) {
                        if let Ok(mut out) = results.lock() {
                            out.push((url, image));
                        }
                    }
                }
            });
        }
    });
    results.into_inner().unwrap_or_default()
}

fn request(url: &str, accept: &str) -> Result<(String, ureq::Response), String> {
    request_with_timeout(url, accept, Duration::from_secs(15))
}

fn request_with_timeout(
    url: &str,
    accept: &str,
    timeout: Duration,
) -> Result<(String, ureq::Response), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(timeout)
        .redirects(8)
        .build();
    let response = agent
        .get(url)
        .set(
            "User-Agent",
            "rust-browser/0.1 (from-scratch; +https://github.com/AlexanderNyr/AuraLite-Browser)",
        )
        .set("Accept", accept)
        .call()
        .map_err(|e| format!("Network error: {}", e))?;

    let final_url = response
        .header("X-Final-Url")
        .map(|s| s.to_string())
        .unwrap_or_else(|| url.to_string());
    // ureq follows redirects internally; the effective URL is not exposed
    // in 2.x, so we keep the requested URL unless the server told us.
    Ok((final_url, response))
}

fn fetch_text(url: &str) -> Result<(String, String), String> {
    let (final_url, response) = request(url, "text/html, text/css, */*")?;
    let body = response
        .into_string()
        .map_err(|e| format!("Read error: {}", e))?;
    Ok((final_url, body))
}

/// Fetch raw bytes (images). Returns `(final_url, bytes)`.
pub fn fetch_bytes(url: &str) -> Result<(String, Vec<u8>), String> {
    fetch_bytes_with_timeout(url, Duration::from_secs(15))
}

fn fetch_bytes_with_timeout(url: &str, timeout: Duration) -> Result<(String, Vec<u8>), String> {
    let (final_url, response) = request_with_timeout(url, "image/*,*/*", timeout)?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_IMAGE_BYTES)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Read error: {}", e))?;
    if bytes.is_empty() {
        return Err("empty response".to_string());
    }
    Ok((final_url, bytes))
}

/// Turn user input into a navigable URL.
/// - `example.com` -> `https://example.com`
/// - `https://example.com` stays as-is
/// - `localhost:8080` -> `http://localhost:8080`
pub fn normalize_url(input: &str) -> String {
    let input = input.trim();
    if input.is_empty() {
        return String::new();
    }
    if input.starts_with("http://") || input.starts_with("https://") {
        return input.to_string();
    }
    if input.starts_with("localhost")
        || input.starts_with("127.0.0.1")
        || input.starts_with("[::1]")
    {
        return format!("http://{}", input);
    }
    // Bare IP or host:port without scheme -> http for local, https otherwise.
    if input
        .split('/')
        .next()
        .map(|h| h.contains(':'))
        .unwrap_or(false)
    {
        return format!("http://{}", input);
    }
    format!("https://{}", input)
}

/// Resolve `href` against `base`. Returns `None` for non-navigable schemes
/// (`mailto:`, `javascript:`, `#fragment`-only links are kept as same-page).
pub fn resolve_url(base: &str, href: &str) -> Option<String> {
    let href = href.trim();
    if href.is_empty() {
        return None;
    }
    if href.starts_with("javascript:") || href.starts_with("mailto:") || href.starts_with("tel:") {
        return None;
    }
    let base_url = Url::parse(base).ok()?;
    let resolved = base_url.join(href).ok()?;
    if resolved.scheme() != "http" && resolved.scheme() != "https" {
        return None;
    }
    Some(resolved.to_string())
}

/// Extract a human-readable host for the tab strip / status bar.
pub fn host_of(url: &str) -> String {
    Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(|s| s.to_string()))
        .unwrap_or_else(|| url.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_urls() {
        assert_eq!(normalize_url("example.com"), "https://example.com");
        assert_eq!(
            normalize_url("https://example.com/x"),
            "https://example.com/x"
        );
        assert_eq!(normalize_url("localhost:3000"), "http://localhost:3000");
        assert_eq!(normalize_url("  example.com/a  "), "https://example.com/a");
    }

    #[test]
    fn resolves_relative_links() {
        let base = "https://example.com/docs/page.html";
        assert_eq!(
            resolve_url(base, "/about").as_deref(),
            Some("https://example.com/about")
        );
        assert_eq!(
            resolve_url(base, "other.html").as_deref(),
            Some("https://example.com/docs/other.html")
        );
        assert_eq!(resolve_url(base, "mailto:a@b.c"), None);
        assert_eq!(resolve_url(base, "javascript:void(0)"), None);
    }
}
