//! rust-browser: a minimal web browser written from scratch in Rust.
//!
//! Architecture (each module is independently testable):
//!
//! ```text
//! net.rs      HTTP/HTTPS fetching (ureq + rustls), URL handling
//! html.rs     HTML tokenizer + DOM builder (from scratch)
//! css.rs      CSS parser, cascade, computed styles (from scratch)
//! layout.rs   block layout + inline formatting / word wrap
//! render.rs   font rasterisation (fontdue) + framebuffer drawing
//! browser.rs  tabs, history, async loading state
//! app.rs      winit window, toolbar/tab-strip UI, input handling
//! ```
//!
//! The GUI is a single `softbuffer` framebuffer: the browser chrome and
//! the page are both drawn with the same software renderer, so there is
//! no GPU dependency and the whole pipeline can be unit-tested headlessly.

pub mod app;
pub mod browser;
pub mod css;
pub mod html;
pub mod layout;
pub mod net;
pub mod render;
