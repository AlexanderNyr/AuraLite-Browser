//! Browser state: tabs, per-tab history, async page loading.
//!
//! Each tab owns its history stack and scroll position. Page loads run on
//! a background thread; the UI polls the channel each frame and applies
//! the result when it arrives (see `Tab::poll_load`).

use crate::css::Stylesheet;
use crate::html::Document;
use crate::image::ImageStore;
use crate::layout::LayoutBox;
use crate::net::{fetch_page, FetchedPage};
use std::sync::mpsc::{channel, Receiver};

/// Built-in start page shown on launch / new tab.
pub const START_PAGE_URL: &str = "about:home";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TabId(pub u64);

/// Result of a background fetch, delivered to the UI thread.
pub struct LoadResult {
    pub tab_id: TabId,
    pub url: String,
    pub page: Result<FetchedPage, String>,
}

/// Messages a worker thread sends to the UI thread.
enum LoadMsg {
    /// HTML + CSS are ready: the page can be shown right away.
    Page(LoadResult),
    /// Images finished downloading after the page was already on screen.
    Images {
        images: Vec<(String, crate::image::DecodedImage)>,
    },
}

/// One browser tab.
pub struct Tab {
    pub id: TabId,
    pub url: String,
    pub title: String,
    pub history: Vec<String>,
    pub history_index: usize,
    pub scroll_y: f32,
    pub document: Option<Document>,
    pub stylesheet: Stylesheet,
    pub layout: Option<LayoutBox>,
    /// Decoded images belonging to the current page.
    pub images: ImageStore,
    pub status: String,
    pub loading: bool,
    pub error: Option<String>,
    pending: Option<Receiver<LoadMsg>>,
}

impl Tab {
    pub fn new(id: TabId) -> Self {
        Self {
            id,
            url: START_PAGE_URL.to_string(),
            title: "New Tab".to_string(),
            history: vec![START_PAGE_URL.to_string()],
            history_index: 0,
            scroll_y: 0.0,
            document: None,
            stylesheet: Stylesheet::default(),
            layout: None,
            images: ImageStore::default(),
            status: "Ready".to_string(),
            loading: false,
            error: None,
            pending: None,
        }
    }

    pub fn can_go_back(&self) -> bool {
        self.history_index > 0
    }

    pub fn can_go_forward(&self) -> bool {
        self.history_index + 1 < self.history.len()
    }

    /// Navigate to `url`, pushing a new history entry.
    /// Returns the URL actually requested (after normalisation).
    pub fn navigate(&mut self, url: &str) -> String {
        let url = crate::net::normalize_url(url);
        if url.is_empty() {
            return String::new();
        }
        // Truncate forward history, then push.
        self.history.truncate(self.history_index + 1);
        self.history.push(url.clone());
        self.history_index = self.history.len() - 1;
        self.start_load(&url);
        url
    }

    pub fn go_back(&mut self) -> bool {
        if !self.can_go_back() {
            return false;
        }
        self.history_index -= 1;
        let url = self.history[self.history_index].clone();
        self.start_load(&url);
        true
    }

    pub fn go_forward(&mut self) -> bool {
        if !self.can_go_forward() {
            return false;
        }
        self.history_index += 1;
        let url = self.history[self.history_index].clone();
        self.start_load(&url);
        true
    }

    pub fn reload(&mut self) {
        let url = self.url.clone();
        if url != START_PAGE_URL {
            self.start_load(&url);
        }
    }

    fn start_load(&mut self, url: &str) {
        self.url = url.to_string();
        self.scroll_y = 0.0;
        self.error = None;
        self.loading = true;
        self.status = format!("Loading {}…", url);
        if url == START_PAGE_URL {
            self.loading = false;
            self.document = None;
            self.layout = None;
            self.images = ImageStore::default();
            self.title = "New Tab".to_string();
            self.status = "Ready".to_string();
            self.pending = None;
            return;
        }
        let (tx, rx) = channel();
        let tab_id = self.id;
        let url_owned = url.to_string();
        std::thread::spawn(move || {
            let dbg = std::env::var_os("RUST_BROWSER_DEBUG_LOAD").is_some();
            if dbg {
                eprintln!("[load] start {url_owned}");
            }
            let page = fetch_page(&url_owned);
            if dbg {
                eprintln!("[load] fetched: ok={}", page.is_ok());
            }
            // Show the document as soon as it is ready...
            let image_urls = match &page {
                Ok(p) => p.image_urls.clone(),
                Err(_) => Vec::new(),
            };
            let _ = tx.send(LoadMsg::Page(LoadResult {
                tab_id,
                url: url_owned,
                page,
            }));
            // ...then keep loading its images in the background.
            if dbg {
                eprintln!("[load] sent page msg, {} images to go", image_urls.len());
            }
            if !image_urls.is_empty() {
                let images = crate::net::fetch_images(image_urls);
                if !images.is_empty() {
                    let _ = tx.send(LoadMsg::Images { images });
                }
            }
        });
        self.pending = Some(rx);
    }

    /// Poll the background fetch. Returns `true` if a load finished.
    pub fn poll_load(&mut self) -> bool {
        let Some(rx) = &self.pending else {
            return false;
        };
        match rx.try_recv() {
            Ok(LoadMsg::Images { images }) => {
                // Page is already visible; drop the images in and re-lay out
                // so boxes pick up their real sizes.
                let base = self.url.clone();
                self.images.set_base_url(base);
                for (url, image) in images {
                    self.images.insert(url, image);
                }
                self.layout = None;
                self.status = format!("Loaded {} ({} images)", self.url, self.images.len());
                true
            }
            Ok(LoadMsg::Page(result)) => {
                self.loading = false;
                match result.page {
                    Ok(page) => {
                        self.apply_page(page);
                    }
                    Err(e) => {
                        self.error = Some(e.clone());
                        self.status = e;
                        self.document = None;
                        self.layout = None;
                        self.images = ImageStore::default();
                        self.title = "Error".to_string();
                    }
                }
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                self.loading = false;
                false
            }
        }
    }

    fn apply_page(&mut self, page: FetchedPage) {
        let doc = crate::html::parse(&page.html);
        self.title = if doc.title.is_empty() {
            crate::net::host_of(&page.final_url)
        } else {
            doc.title.clone()
        };
        // UA stylesheet first, then linked CSS, then inline <style>.
        let mut sheet = crate::css::ua_stylesheet();
        for css in &page.linked_css {
            sheet.extend(Stylesheet::parse(css));
        }
        for css in doc.style_texts() {
            sheet.extend(Stylesheet::parse(&css));
        }
        self.document = Some(doc);
        self.stylesheet = sheet;
        self.images = ImageStore::new(page.final_url.clone());
        self.layout = None; // relaid out by the UI with the real viewport width
        self.status = format!("Loaded {}", page.final_url);
        self.url = page.final_url;
    }

    /// (Re)build the layout tree for the current viewport width.
    pub fn relayout(&mut self, viewport_width: f32, fonts: &crate::render::FontSet) {
        if let Some(doc) = &self.document {
            self.layout = Some(crate::layout::layout_document(
                doc,
                &self.stylesheet,
                viewport_width,
                fonts,
                &self.images,
            ));
        }
    }

    /// Is a page or its images still arriving? The UI uses this to keep
    /// polling the event loop instead of sleeping.
    pub fn is_busy(&self) -> bool {
        self.pending.is_some() || self.loading
    }

    /// Total scrollable height: content height minus what fits on screen.
    pub fn max_scroll(&self, viewport_height: f32) -> f32 {
        match &self.layout {
            Some(l) => (l.height - viewport_height).max(0.0),
            None => 0.0,
        }
    }

    /// Scroll by `delta` px, clamped to `[0, max_scroll]`.
    pub fn scroll_by(&mut self, delta: f32, viewport_height: f32) {
        let limit = self.max_scroll(viewport_height);
        self.scroll_y = (self.scroll_y + delta).clamp(0.0, limit);
    }

    /// Jump to an absolute scroll offset, clamped the same way.
    pub fn scroll_to(&mut self, y: f32, viewport_height: f32) {
        let limit = self.max_scroll(viewport_height);
        self.scroll_y = y.clamp(0.0, limit);
    }

    pub fn display_url(&self) -> String {
        if self.url == START_PAGE_URL {
            String::new()
        } else {
            self.url.clone()
        }
    }
}

impl Browser {
    /// Any tab still loading? Drives the event loop's idle behaviour.
    pub fn is_busy(&self) -> bool {
        self.tabs.iter().any(|t| t.is_busy())
    }
}

/// The whole browser: a list of tabs plus the active one.
pub struct Browser {
    pub tabs: Vec<Tab>,
    pub active: usize,
    next_id: u64,
}

impl Browser {
    pub fn new() -> Self {
        let mut browser = Self {
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
        };
        browser.new_tab();
        browser
    }

    pub fn active_tab(&self) -> &Tab {
        &self.tabs[self.active]
    }

    pub fn active_tab_mut(&mut self) -> &mut Tab {
        &mut self.tabs[self.active]
    }

    pub fn new_tab(&mut self) -> usize {
        let id = TabId(self.next_id);
        self.next_id += 1;
        self.tabs.push(Tab::new(id));
        self.active = self.tabs.len() - 1;
        self.active
    }

    /// Close a tab by index. Always keeps at least one tab.
    /// Returns the new active index.
    pub fn close_tab(&mut self, index: usize) -> usize {
        if self.tabs.len() <= 1 || index >= self.tabs.len() {
            return self.active;
        }
        self.tabs.remove(index);
        if self.active >= self.tabs.len() {
            self.active = self.tabs.len() - 1;
        } else if index < self.active {
            self.active -= 1;
        }
        self.active
    }

    pub fn activate(&mut self, index: usize) {
        if index < self.tabs.len() {
            self.active = index;
        }
    }

    pub fn next_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active = (self.active + 1) % self.tabs.len();
        }
    }

    pub fn prev_tab(&mut self) {
        if !self.tabs.is_empty() {
            self.active = if self.active == 0 {
                self.tabs.len() - 1
            } else {
                self.active - 1
            };
        }
    }

    /// Poll every tab for finished loads. Returns true if anything changed.
    pub fn poll_all(&mut self) -> bool {
        let mut changed = false;
        for tab in &mut self.tabs {
            if tab.poll_load() {
                changed = true;
            }
        }
        changed
    }
}

impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn long_page_tab() -> Tab {
        let mut tab = Tab::new(TabId(1));
        let html = format!("<body>{}</body>", "<p>line of text</p>".repeat(80));
        tab.document = Some(crate::html::parse(&html));
        tab.stylesheet = crate::css::ua_stylesheet();
        tab.relayout(400.0, &crate::render::FontSet::load());
        tab
    }

    #[test]
    fn idle_tab_is_not_busy() {
        assert!(!Tab::new(TabId(1)).is_busy());
    }

    #[test]
    fn scroll_is_clamped_to_content() {
        let mut tab = long_page_tab();
        let viewport = 400.0;
        let limit = tab.max_scroll(viewport);
        assert!(limit > 0.0, "test page should overflow");

        tab.scroll_by(-100.0, viewport);
        assert_eq!(tab.scroll_y, 0.0, "cannot scroll above the top");

        tab.scroll_by(limit * 4.0, viewport);
        assert_eq!(tab.scroll_y, limit, "cannot scroll past the bottom");

        tab.scroll_to(10.0, viewport);
        assert_eq!(tab.scroll_y, 10.0);

        tab.scroll_to(limit + 500.0, viewport);
        assert_eq!(tab.scroll_y, limit);
    }

    #[test]
    fn short_page_cannot_scroll() {
        let mut tab = Tab::new(TabId(2));
        tab.document = Some(crate::html::parse("<body><p>hi</p></body>"));
        tab.stylesheet = crate::css::ua_stylesheet();
        tab.relayout(800.0, &crate::render::FontSet::load());
        assert_eq!(tab.max_scroll(600.0), 0.0);
        tab.scroll_by(200.0, 600.0);
        assert_eq!(tab.scroll_y, 0.0);
    }
}
