//! Networking: HTTP/HTTPS fetching with `ureq` (rustls for TLS).
//!
//! Fetching runs on a background thread (see `browser::Tab::navigate`)
//! so the UI thread never blocks on the network.

use std::time::Duration;
use url::Url;

/// A fully fetched page: HTML plus any linked stylesheets.
#[derive(Debug, Clone)]
pub struct FetchedPage {
    pub final_url: String,
    pub html: String,
    /// Contents of `<link rel="stylesheet">` files, in document order.
    pub linked_css: Vec<String>,
}

/// Fetch `url` (following redirects) and its linked stylesheets.
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
    Ok(FetchedPage {
        final_url,
        html,
        linked_css,
    })
}

fn fetch_text(url: &str) -> Result<(String, String), String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(15))
        .build();
    let response = agent
        .get(url)
        .set(
            "User-Agent",
            "rust-browser/0.1 (from-scratch; +https://github.com/example/rust-browser)",
        )
        .set("Accept", "text/html, text/css, */*")
        .call()
        .map_err(|e| format!("Network error: {}", e))?;

    let final_url = response
        .header("X-Final-Url")
        .map(|s| s.to_string())
        .unwrap_or_else(|| url.to_string());
    // ureq follows redirects internally; the effective URL is not exposed
    // in 2.x, so we keep the requested URL unless the server told us.
    let body = response
        .into_string()
        .map_err(|e| format!("Read error: {}", e))?;
    Ok((final_url, body))
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
