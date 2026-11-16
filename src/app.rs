//! GUI shell built on `winit` + `softbuffer`.
//!
//! Layout (top to bottom):
//! ```text
//! +--------------------------------------------------+
//! | [Tab 1] [Tab 2] [+]                            |  tab bar
//! +--------------------------------------------------+
//! | [Back] [Fwd] [Reload] [ address bar........ ] [Go] | toolbar
//! +--------------------------------------------------+
//! |                                                  |
//! |                  page viewport                   |
//! |                                                  |
//! +--------------------------------------------------+
//! | status / hovered link                   v0.1.0   | status bar
//! +--------------------------------------------------+
//! ```
//!
//! Shortcuts: Ctrl+T new tab, Ctrl+W close, Ctrl+L focus address,
//! Ctrl+R / F5 reload, Alt+Left/Right back/forward, Ctrl+Tab next tab.

use crate::browser::{Browser, Tab};
use crate::css::Color;
use crate::layout::hit_test;
use crate::render::{render_message, render_page, FontSet, Framebuffer};
use std::num::NonZeroU32;
use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowAttributes, WindowId};

const COL_SCROLL_TRACK: Color = Color::rgb(240, 240, 242);
const COL_SCROLL_THUMB: Color = Color::rgb(170, 172, 178);

const TAB_BAR_H: f32 = 34.0;
const TOOLBAR_H: f32 = 46.0;
const STATUS_H: f32 = 24.0;
const TAB_W: f32 = 168.0;

const COL_TAB_BAR: Color = Color::rgb(222, 225, 230);
const COL_TOOLBAR: Color = Color::rgb(241, 243, 244);
const COL_TAB_ACTIVE: Color = Color::rgb(255, 255, 255);
const COL_TAB_INACTIVE: Color = Color::rgb(218, 220, 224);
const COL_BORDER: Color = Color::rgb(180, 180, 180);
const COL_TEXT: Color = Color::rgb(32, 33, 36);
const COL_TEXT_DIM: Color = Color::rgb(95, 99, 104);
const COL_ACCENT: Color = Color::rgb(26, 115, 232);
const COL_BUTTON: Color = Color::rgb(255, 255, 255);
const COL_BUTTON_DISABLED: Color = Color::rgb(232, 234, 237);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChromeHit {
    Tab(usize),
    NewTab,
    CloseTab(usize),
    Back,
    Forward,
    Reload,
    AddressBar,
    Go,
    Viewport,
    None,
}

pub struct App {
    window: Option<Arc<Window>>,
    context: Option<softbuffer::Context<Arc<Window>>>,
    surface: Option<softbuffer::Surface<Arc<Window>, Arc<Window>>>,
    fonts: FontSet,
    browser: Browser,
    address_text: String,
    address_cursor: usize, // char index
    address_focused: bool,
    modifiers: ModifiersState,
    mouse: (f32, f32),
    hover_link: Option<String>,
    size: (u32, u32),
    needs_relayout: bool,
}

impl App {
    pub fn new() -> Self {
        Self::with_url(None)
    }

    /// Build the app, optionally opening `url` in the first tab
    /// (`cargo run -- example.com`).
    pub fn with_url(url: Option<String>) -> Self {
        let mut browser = Browser::new();
        if let Some(url) = url.filter(|u| !u.trim().is_empty()) {
            browser.active_tab_mut().navigate(&url);
        }
        Self {
            window: None,
            context: None,
            surface: None,
            fonts: FontSet::load(),
            browser,
            address_text: String::new(),
            address_cursor: 0,
            address_focused: false,
            modifiers: ModifiersState::empty(),
            mouse: (0.0, 0.0),
            hover_link: None,
            size: (1100, 750),
            needs_relayout: true,
        }
    }

    fn viewport_rect(&self) -> (f32, f32, f32, f32) {
        let (w, h) = (self.size.0 as f32, self.size.1 as f32);
        let y = TAB_BAR_H + TOOLBAR_H;
        (0.0, y, w, (h - y - STATUS_H).max(0.0))
    }

    fn tab_rect(&self, index: usize) -> (f32, f32, f32, f32) {
        (
            8.0 + index as f32 * TAB_W,
            3.0,
            TAB_W - 8.0,
            TAB_BAR_H - 6.0,
        )
    }

    fn new_tab_rect(&self) -> (f32, f32, f32, f32) {
        let x = 8.0 + self.browser.tabs.len() as f32 * TAB_W;
        (x, 3.0, 32.0, TAB_BAR_H - 6.0)
    }

    fn back_rect(&self) -> (f32, f32, f32, f32) {
        (8.0, TAB_BAR_H + 7.0, 64.0, TOOLBAR_H - 14.0)
    }

    fn forward_rect(&self) -> (f32, f32, f32, f32) {
        (76.0, TAB_BAR_H + 7.0, 64.0, TOOLBAR_H - 14.0)
    }

    fn reload_rect(&self) -> (f32, f32, f32, f32) {
        (144.0, TAB_BAR_H + 7.0, 64.0, TOOLBAR_H - 14.0)
    }

    fn address_rect(&self) -> (f32, f32, f32, f32) {
        let w = self.size.0 as f32;
        (
            216.0,
            TAB_BAR_H + 7.0,
            (w - 216.0 - 76.0).max(40.0),
            TOOLBAR_H - 14.0,
        )
    }

    fn go_rect(&self) -> (f32, f32, f32, f32) {
        let w = self.size.0 as f32;
        (w - 68.0, TAB_BAR_H + 7.0, 60.0, TOOLBAR_H - 14.0)
    }

    fn in_rect(x: f32, y: f32, r: (f32, f32, f32, f32)) -> bool {
        x >= r.0 && x < r.0 + r.2 && y >= r.1 && y < r.1 + r.3
    }

    fn chrome_hit(&self, x: f32, y: f32) -> ChromeHit {
        for i in 0..self.browser.tabs.len() {
            let r = self.tab_rect(i);
            if Self::in_rect(x, y, r) {
                // Close button is the rightmost 24px of the tab.
                if x > r.0 + r.2 - 24.0 && self.browser.tabs.len() > 1 {
                    return ChromeHit::CloseTab(i);
                }
                return ChromeHit::Tab(i);
            }
        }
        if Self::in_rect(x, y, self.new_tab_rect()) {
            return ChromeHit::NewTab;
        }
        if Self::in_rect(x, y, self.back_rect()) {
            return ChromeHit::Back;
        }
        if Self::in_rect(x, y, self.forward_rect()) {
            return ChromeHit::Forward;
        }
        if Self::in_rect(x, y, self.reload_rect()) {
            return ChromeHit::Reload;
        }
        if Self::in_rect(x, y, self.address_rect()) {
            return ChromeHit::AddressBar;
        }
        if Self::in_rect(x, y, self.go_rect()) {
            return ChromeHit::Go;
        }
        if Self::in_rect(x, y, self.viewport_rect()) {
            return ChromeHit::Viewport;
        }
        ChromeHit::None
    }

    fn sync_address_bar(&mut self) {
        if !self.address_focused {
            self.address_text = self.browser.active_tab().display_url();
            self.address_cursor = self.address_text.chars().count();
        }
    }

    fn navigate_address(&mut self) {
        let url = self.address_text.clone();
        if url.trim().is_empty() {
            return;
        }
        self.browser.active_tab_mut().navigate(&url);
        self.address_focused = false;
        self.sync_address_bar();
        self.needs_relayout = true;
        self.request_redraw();
    }

    fn request_redraw(&self) {
        if let Some(w) = &self.window {
            w.request_redraw();
        }
    }

    fn ensure_layout(&mut self) {
        let dbg = std::env::var_os("RUST_BROWSER_DEBUG_LOAD").is_some();
        if self.needs_relayout {
            let t = std::time::Instant::now();
            if dbg {
                eprintln!("[ui] relayout for width {}", self.viewport_rect().2);
            }
            let vw = self.viewport_rect().2;
            let fonts_ptr: *const FontSet = &self.fonts;
            // SAFETY: fonts outlives this call; we only read through the ptr
            // while holding &mut self.browser (disjoint fields).
            let fonts = unsafe { &*fonts_ptr };
            self.browser.active_tab_mut().relayout(vw, fonts);
            if dbg {
                eprintln!("[ui] relayout took {:?}", t.elapsed());
            }
            self.needs_relayout = false;
        }
    }

    fn update_hover(&mut self) {
        let (mx, my) = self.mouse;
        let (_, vy, _, _) = self.viewport_rect();
        self.hover_link = None;
        if my < vy {
            return;
        }
        let tab = self.browser.active_tab();
        if let Some(layout) = &tab.layout {
            if let Some(hit) = hit_test(layout, mx, my - vy + tab.scroll_y) {
                self.hover_link = hit.link;
            }
        }
    }

    fn on_click(&mut self, x: f32, y: f32) {
        match self.chrome_hit(x, y) {
            ChromeHit::Tab(i) => {
                self.browser.activate(i);
                self.address_focused = false;
                self.sync_address_bar();
                self.needs_relayout = true;
            }
            ChromeHit::CloseTab(i) => {
                self.browser.close_tab(i);
                self.sync_address_bar();
                self.needs_relayout = true;
            }
            ChromeHit::NewTab => {
                self.browser.new_tab();
                self.sync_address_bar();
                self.needs_relayout = true;
            }
            ChromeHit::Back => {
                if self.browser.active_tab_mut().go_back() {
                    self.sync_address_bar();
                    self.needs_relayout = true;
                }
            }
            ChromeHit::Forward => {
                if self.browser.active_tab_mut().go_forward() {
                    self.sync_address_bar();
                    self.needs_relayout = true;
                }
            }
            ChromeHit::Reload => {
                self.browser.active_tab_mut().reload();
                self.needs_relayout = true;
            }
            ChromeHit::AddressBar => {
                self.address_focused = true;
                self.address_cursor = self.address_text.chars().count();
            }
            ChromeHit::Go => {
                self.navigate_address();
            }
            ChromeHit::Viewport => {
                self.address_focused = false;
                let (_, vy, _, _) = self.viewport_rect();
                let tab = self.browser.active_tab();
                let scroll = tab.scroll_y;
                let url = tab.url.clone();
                let link = tab
                    .layout
                    .as_ref()
                    .and_then(|l| hit_test(l, x, y - vy + scroll))
                    .and_then(|h| h.link);
                if let Some(href) = link {
                    if let Some(resolved) = crate::net::resolve_url(&url, &href) {
                        self.browser.active_tab_mut().navigate(&resolved);
                        self.sync_address_bar();
                        self.needs_relayout = true;
                    }
                }
            }
            ChromeHit::None => {
                self.address_focused = false;
            }
        }
        self.request_redraw();
    }

    fn on_key(&mut self, key: &Key, text: Option<&str>, pressed: bool) {
        if !pressed {
            return;
        }
        let ctrl = self.modifiers.control_key();
        let alt = self.modifiers.alt_key();

        if self.address_focused {
            match key {
                Key::Named(NamedKey::Enter) => self.navigate_address(),
                Key::Named(NamedKey::Escape) => {
                    self.address_focused = false;
                    self.sync_address_bar();
                }
                Key::Named(NamedKey::Backspace) => {
                    let byte = char_to_byte(&self.address_text, self.address_cursor);
                    if self.address_cursor > 0 && byte > 0 {
                        let prev = self.address_text[..byte].chars().last().unwrap();
                        self.address_text
                            .replace_range(byte - prev.len_utf8()..byte, "");
                        self.address_cursor -= 1;
                    }
                }
                Key::Named(NamedKey::ArrowLeft) => {
                    self.address_cursor = self.address_cursor.saturating_sub(1);
                }
                Key::Named(NamedKey::ArrowRight) => {
                    let max = self.address_text.chars().count();
                    if self.address_cursor < max {
                        self.address_cursor += 1;
                    }
                }
                Key::Named(NamedKey::Home) => self.address_cursor = 0,
                Key::Named(NamedKey::End) => {
                    self.address_cursor = self.address_text.chars().count()
                }
                Key::Character(_) => {
                    if let Some(t) = text {
                        if !ctrl && !alt {
                            let byte = char_to_byte(&self.address_text, self.address_cursor);
                            self.address_text.insert_str(byte, t);
                            self.address_cursor += t.chars().count();
                        }
                    }
                }
                _ => {}
            }
            self.request_redraw();
            return;
        }

        // Global shortcuts.
        match key {
            Key::Character(c) => {
                let s = c.to_ascii_lowercase();
                if ctrl && s == "t" {
                    self.browser.new_tab();
                    self.sync_address_bar();
                    self.needs_relayout = true;
                } else if ctrl && s == "w" {
                    let active = self.browser.active;
                    self.browser.close_tab(active);
                    self.sync_address_bar();
                    self.needs_relayout = true;
                } else if ctrl && s == "l" {
                    self.address_focused = true;
                    self.address_cursor = self.address_text.chars().count();
                } else if ctrl && s == "r" {
                    self.browser.active_tab_mut().reload();
                    self.needs_relayout = true;
                } else if ctrl && s == "\t" {
                    self.browser.next_tab();
                    self.sync_address_bar();
                    self.needs_relayout = true;
                }
            }
            Key::Named(NamedKey::F5) => {
                self.browser.active_tab_mut().reload();
                self.needs_relayout = true;
            }
            // Page scrolling (the address bar has its own Home/End handling,
            // so this branch only runs when it is not focused).
            Key::Named(NamedKey::PageDown) => self.page_scroll(0.9),
            Key::Named(NamedKey::PageUp) => self.page_scroll(-0.9),
            Key::Named(NamedKey::Space) => self.page_scroll(0.9),
            Key::Named(NamedKey::Home) => self.scroll_to_absolute(0.0),
            Key::Named(NamedKey::End) => self.scroll_to_bottom(),
            Key::Named(NamedKey::ArrowDown) => self.page_scroll_step(60.0),
            Key::Named(NamedKey::ArrowUp) => self.page_scroll_step(-60.0),
            Key::Named(NamedKey::ArrowLeft) if alt => {
                if self.browser.active_tab_mut().go_back() {
                    self.sync_address_bar();
                    self.needs_relayout = true;
                }
            }
            Key::Named(NamedKey::ArrowRight)
                if alt && self.browser.active_tab_mut().go_forward() =>
            {
                self.sync_address_bar();
                self.needs_relayout = true;
            }
            _ => {}
        }
        self.request_redraw();
    }

    /// Scroll by a fraction of the viewport height (page up / page down).
    fn page_scroll(&mut self, pages: f32) {
        if self.address_focused {
            return;
        }
        let vh = self.viewport_rect().3;
        let tab = self.browser.active_tab_mut();
        tab.scroll_by(pages * vh, vh);
    }

    /// Scroll by an exact pixel amount.
    fn page_scroll_step(&mut self, pixels: f32) {
        if self.address_focused {
            return;
        }
        let vh = self.viewport_rect().3;
        self.browser.active_tab_mut().scroll_by(pixels, vh);
    }

    fn scroll_to_absolute(&mut self, y: f32) {
        if self.address_focused {
            return;
        }
        let vh = self.viewport_rect().3;
        self.browser.active_tab_mut().scroll_to(y, vh);
    }

    fn scroll_to_bottom(&mut self) {
        if self.address_focused {
            return;
        }
        let vh = self.viewport_rect().3;
        let limit = self.browser.active_tab().max_scroll(vh);
        self.browser.active_tab_mut().scroll_to(limit, vh);
    }

    fn render(&mut self) {
        let (w, h) = self.size;
        if w == 0 || h == 0 || self.surface.is_none() {
            return;
        }
        self.ensure_layout();
        self.update_hover();

        // Draw into an off-screen buffer first, then blit into the window
        // surface. Keeps the drawing code free of borrow conflicts with
        // `&mut self` helpers and makes the frame easy to snapshot in tests.
        let fb_w = w as usize;
        let fb_h = h as usize;
        let mut pixels = vec![0u32; fb_w * fb_h];
        let fonts = &self.fonts;
        {
            let mut fb = Framebuffer::new(&mut pixels, fb_w, fb_h);

            // --- Tab bar ---
            fb.fill_rect(0.0, 0.0, fb_w as f32, TAB_BAR_H, COL_TAB_BAR);
            for i in 0..self.browser.tabs.len() {
                let r = self.tab_rect(i);
                let active = i == self.browser.active;
                fb.fill_rect(
                    r.0,
                    r.1,
                    r.2,
                    r.3,
                    if active {
                        COL_TAB_ACTIVE
                    } else {
                        COL_TAB_INACTIVE
                    },
                );
                fb.stroke_rect(r.0, r.1, r.2, r.3, COL_BORDER);
                let title = truncate(&self.browser.tabs[i].title, fonts, 13.0, r.2 - 32.0);
                fb.draw_text(
                    r.0 + 8.0,
                    r.1 + 7.0,
                    &title,
                    13.0,
                    COL_TEXT,
                    active,
                    false,
                    fonts,
                );
                if self.browser.tabs.len() > 1 {
                    fb.draw_text(
                        r.0 + r.2 - 20.0,
                        r.1 + 7.0,
                        "×",
                        13.0,
                        COL_TEXT_DIM,
                        false,
                        false,
                        fonts,
                    );
                }
            }
            let nr = self.new_tab_rect();
            fb.fill_rect(nr.0, nr.1, nr.2, nr.3, COL_TAB_INACTIVE);
            fb.stroke_rect(nr.0, nr.1, nr.2, nr.3, COL_BORDER);
            fb.draw_text_centered(nr, "+", 16.0, COL_TEXT, false, fonts);

            // --- Toolbar ---
            let ty = TAB_BAR_H;
            fb.fill_rect(0.0, ty, fb_w as f32, TOOLBAR_H, COL_TOOLBAR);
            fb.fill_rect(0.0, ty + TOOLBAR_H - 1.0, fb_w as f32, 1.0, COL_BORDER);

            let tab = self.browser.active_tab();
            let can_back = tab.can_go_back();
            let can_fwd = tab.can_go_forward();

            draw_button(&mut fb, self.back_rect(), "← Back", can_back, fonts);
            draw_button(&mut fb, self.forward_rect(), "Fwd →", can_fwd, fonts);
            draw_button(&mut fb, self.reload_rect(), "⟳ Reload", true, fonts);

            // Address bar.
            let ar = self.address_rect();
            fb.fill_rect(ar.0, ar.1, ar.2, ar.3, Color::rgb(255, 255, 255));
            fb.stroke_rect(
                ar.0,
                ar.1,
                ar.2,
                ar.3,
                if self.address_focused {
                    COL_ACCENT
                } else {
                    COL_BORDER
                },
            );
            let shown = self.address_text.clone();
            let display = if shown.is_empty() && !self.address_focused {
                "Search or enter address".to_string()
            } else {
                shown
            };
            let color = if display == "Search or enter address" {
                COL_TEXT_DIM
            } else {
                COL_TEXT
            };
            // Keep the tail visible when the text is too long.
            let text_w = fonts.width(&display, 14.0, false, false);
            let max_w = ar.2 - 16.0;
            let offset = if text_w > max_w { text_w - max_w } else { 0.0 };
            fb.draw_text(
                ar.0 + 8.0 - offset,
                ar.1 + 8.0,
                &display,
                14.0,
                color,
                false,
                false,
                fonts,
            );
            if self.address_focused {
                // Text cursor.
                let before: String = display.chars().take(self.address_cursor).collect();
                let cx = ar.0 + 8.0 - offset + fonts.width(&before, 14.0, false, false);
                fb.fill_rect(cx, ar.1 + 6.0, 1.5, ar.3 - 12.0, COL_TEXT);
            }
            draw_button(&mut fb, self.go_rect(), "Go", true, fonts);

            // --- Viewport ---
            let vp = self.viewport_rect();
            let tab = self.browser.active_tab();
            if let Some(layout) = &tab.layout {
                render_page(&mut fb, layout, tab.scroll_y, vp, fonts, &tab.images);
            } else if let Some(err) = &tab.error {
                render_message(
                    &mut fb,
                    vp,
                    "Can't reach this page",
                    &[err.as_str(), "", "Check the address and try again."],
                    fonts,
                );
            } else if tab.loading {
                render_message(&mut fb, vp, "Loading…", &[&tab.status], fonts);
            } else {
                render_message(
                    &mut fb,
                    vp,
                    "Rust Browser",
                    &[
                        "Type an address above and press Enter.",
                        "",
                        "Shortcuts: Ctrl+T new tab · Ctrl+W close · Ctrl+L address",
                        "Ctrl+R reload · Alt+←/→ back/forward · mouse wheel scrolls",
                    ],
                    fonts,
                );
            }

            // --- Scrollbar ---
            draw_scrollbar(&mut fb, vp, tab);

            // --- Status bar ---
            let sy = fb_h as f32 - STATUS_H;
            fb.fill_rect(0.0, sy, fb_w as f32, STATUS_H, COL_TOOLBAR);
            fb.fill_rect(0.0, sy, fb_w as f32, 1.0, COL_BORDER);
            let status_text = if let Some(link) = &self.hover_link {
                link.clone()
            } else {
                tab.status.clone()
            };
            fb.draw_text(
                8.0,
                sy + 5.0,
                &status_text,
                12.0,
                COL_TEXT_DIM,
                false,
                false,
                fonts,
            );
            let ver = "rust-browser 0.1.0";
            let vw = fonts.width(ver, 12.0, false, false);
            fb.draw_text(
                fb_w as f32 - vw - 8.0,
                sy + 5.0,
                ver,
                12.0,
                COL_TEXT_DIM,
                false,
                false,
                fonts,
            );
        } // end drawing (releases the borrow of `pixels` / `self.fonts`)

        // Blit into the window surface and present.
        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        let buffer_result = surface.buffer_mut();
        let Ok(mut buffer) = buffer_result else {
            return;
        };
        let bw = buffer.width().get() as usize;
        let bh = buffer.height().get() as usize;
        if bw == fb_w && bh == fb_h {
            buffer.copy_from_slice(&pixels);
        } else {
            // Surface size lags a resize: copy row by row, whatever fits.
            let rows = bh.min(fb_h);
            let cols = bw.min(fb_w);
            for y in 0..rows {
                let src = y * fb_w;
                let dst = y * bw;
                buffer[dst..dst + cols].copy_from_slice(&pixels[src..src + cols]);
            }
        }
        if let Err(e) = buffer.present() {
            eprintln!("present error: {:?}", e);
        }
    }
}

/// Overlay scrollbar on the right edge of the viewport. Only shown when the
/// page is taller than the window.
fn draw_scrollbar(fb: &mut Framebuffer, viewport: (f32, f32, f32, f32), tab: &Tab) {
    let (vx, vy, vw, vh) = viewport;
    let Some(layout) = &tab.layout else {
        return;
    };
    let content = layout.height;
    if vh <= 1.0 || content <= vh + 1.0 {
        return;
    }
    const W: f32 = 11.0;
    let track_x = vx + vw - W;
    fb.fill_rect(track_x, vy, W, vh, COL_SCROLL_TRACK);

    let thumb_h = (vh * (vh / content)).max(28.0).min(vh);
    let max_scroll = (content - vh).max(1.0);
    let t = (tab.scroll_y / max_scroll).clamp(0.0, 1.0);
    let thumb_y = vy + t * (vh - thumb_h);
    fb.fill_rect(track_x + 2.0, thumb_y, W - 4.0, thumb_h, COL_SCROLL_THUMB);
}

fn draw_button(
    fb: &mut Framebuffer,
    r: (f32, f32, f32, f32),
    label: &str,
    enabled: bool,
    fonts: &FontSet,
) {
    fb.fill_rect(
        r.0,
        r.1,
        r.2,
        r.3,
        if enabled {
            COL_BUTTON
        } else {
            COL_BUTTON_DISABLED
        },
    );
    fb.stroke_rect(r.0, r.1, r.2, r.3, COL_BORDER);
    fb.draw_text_centered(
        r,
        label,
        13.0,
        if enabled { COL_TEXT } else { COL_TEXT_DIM },
        false,
        fonts,
    );
}

fn truncate(text: &str, fonts: &FontSet, size: f32, max_w: f32) -> String {
    if fonts.width(text, size, false, false) <= max_w {
        return text.to_string();
    }
    let mut out = String::new();
    for c in text.chars() {
        out.push(c);
        if fonts.width(&format!("{}…", out), size, false, false) > max_w {
            out.pop();
            break;
        }
    }
    format!("{}…", out)
}

/// Convert a char index to a byte index in `s`.
fn char_to_byte(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(b, _)| b)
        .unwrap_or(s.len())
}

impl Default for App {
    fn default() -> Self {
        Self::new()
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        event_loop.set_control_flow(ControlFlow::Wait);
        let attrs = WindowAttributes::default()
            .with_title("Rust Browser")
            .with_inner_size(winit::dpi::LogicalSize::new(1100, 750));
        let window = match event_loop.create_window(attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                eprintln!("failed to create window: {:?}", e);
                event_loop.exit();
                return;
            }
        };
        let size = window.inner_size();
        self.size = (size.width, size.height);
        let context = match softbuffer::Context::new(window.clone()) {
            Ok(c) => c,
            Err(e) => {
                eprintln!("failed to create softbuffer context: {:?}", e);
                event_loop.exit();
                return;
            }
        };
        let mut surface = match softbuffer::Surface::new(&context, window.clone()) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("failed to create surface: {:?}", e);
                event_loop.exit();
                return;
            }
        };
        if size.width > 0 && size.height > 0 {
            let _ = surface.resize(
                NonZeroU32::new(size.width).unwrap(),
                NonZeroU32::new(size.height).unwrap(),
            );
        }
        self.context = Some(context);
        self.surface = Some(surface);
        self.window = Some(window);
        self.sync_address_bar();
        self.request_redraw();
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),
            WindowEvent::Resized(size) => {
                self.size = (size.width, size.height);
                if let Some(surface) = &mut self.surface {
                    if size.width > 0 && size.height > 0 {
                        let _ = surface.resize(
                            NonZeroU32::new(size.width).unwrap(),
                            NonZeroU32::new(size.height).unwrap(),
                        );
                    }
                }
                self.needs_relayout = true;
                self.request_redraw();
            }
            WindowEvent::RedrawRequested => {
                self.render();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.mouse = (position.x as f32, position.y as f32);
                self.request_redraw();
            }
            WindowEvent::MouseInput { state, button, .. } => {
                if state == ElementState::Pressed && button == MouseButton::Left {
                    let (x, y) = self.mouse;
                    self.on_click(x, y);
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let dy = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 28.0,
                    MouseScrollDelta::PixelDelta(pos) => pos.y as f32,
                };
                let vh = self.viewport_rect().3;
                self.browser.active_tab_mut().scroll_by(dy, vh);
                self.request_redraw();
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let pressed = event.state == ElementState::Pressed;
                let text = event.text.as_ref().map(|s| s.as_str());
                self.on_key(&event.logical_key, text, pressed);
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
            }
            WindowEvent::Focused(focused) if !focused => {
                self.address_focused = false;
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        // A background load finishes on its own thread and cannot wake a
        // sleeping event loop, so keep a short timeout while anything is in
        // flight (otherwise a slow page would never be picked up).
        if self.browser.is_busy() {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                std::time::Instant::now() + std::time::Duration::from_millis(16),
            ));
        } else {
            event_loop.set_control_flow(ControlFlow::Wait);
        }
        if self.browser.poll_all() {
            self.needs_relayout = true;
            self.sync_address_bar();
            self.request_redraw();
        }
    }
}

/// Run the browser event loop. Called from `main`.
/// Open the browser with an optional start URL.
pub fn run_with_url(url: Option<String>) {
    let event_loop = match winit::event_loop::EventLoop::new() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("failed to create event loop: {:?}", e);
            return;
        }
    };
    let mut app = App::with_url(url);
    if let Err(e) = event_loop.run_app(&mut app) {
        eprintln!("event loop error: {:?}", e);
    }
}

/// Open the browser with no start URL.
pub fn run() {
    run_with_url(None);
}
