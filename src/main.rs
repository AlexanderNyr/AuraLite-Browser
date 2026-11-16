//! rust-browser — entry point.
//!
//! Usage:
//!   cargo run                    # open the start page
//!   cargo run -- example.com     # open a URL (scheme optional)

use std::process::ExitCode;

fn main() -> ExitCode {
    let arg = std::env::args().nth(1);
    match arg.as_deref() {
        Some("-h" | "--help") => {
            println!("rust-browser {}", env!("CARGO_PKG_VERSION"));
            println!();
            println!("Usage: rust-browser [URL]");
            println!();
            println!("  URL   page to open on startup (http:// is assumed if no scheme given)");
            println!();
            println!("Shortcuts: Ctrl+T new tab · Ctrl+W close tab · Ctrl+L address bar");
            println!("           Ctrl+R / F5 reload · Alt+Left/Right history · Ctrl+Tab next tab");
            ExitCode::SUCCESS
        }
        Some("-V" | "--version") => {
            println!("rust-browser {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        _ => {
            rust_browser::app::run_with_url(arg);
            ExitCode::SUCCESS
        }
    }
}
