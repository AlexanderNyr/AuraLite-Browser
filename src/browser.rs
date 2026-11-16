//! Browser state: tabs, per-tab history, async page loading.
//!
//! Each tab owns its history stack and scroll position. Page loads run on
//! a background thread; the UI polls the channel each frame and applies
//! the result when it arrives (see `Tab::poll_load`).

use crate::css::Stylesheet;
use crate::html::Document;
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
    pub status: String,
    pub loading: bool,
    pub error: Option<String>,
    pending: Option<Receiver<LoadResult>>,
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
            self.title = "New Tab".to_string();
            self.status = "Ready".to_string();
            self.pending = None;
            return;
        }
        let (tx, rx) = channel();
        let tab_id = self.id;
        let url_owned = url.to_string();
        std::thread::spawn(move || {
            let page = fetch_page(&url_owned);
            let _ = tx.send(LoadResult {
                tab_id,
                url: url_owned,
                page,
            });
        });
        self.pending = Some(rx);
    }

    /// Poll the background fetch. Returns `true` if a load finished.
    pub fn poll_load(&mut self) -> bool {
        let Some(rx) = &self.pending else {
            return false;
        };
        match rx.try_recv() {
            Ok(result) => {
                self.pending = None;
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
            ));
        }
    }

    pub fn display_url(&self) -> String {
        if self.url == START_PAGE_URL {
            String::new()
        } else {
            self.url.clone()
        }
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
