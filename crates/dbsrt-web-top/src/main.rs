//! WebAssembly browser entry point for interactive dbsrt top TUI dashboard.
//!
//! Compiles to WebAssembly (`wasm32-unknown-unknown`) using Ratzilla DomBackend
//! to render Ratatui components directly in web browsers with zero server-side PHP.
//! Testable logic lives in the library crate; this binary only wires the browser glue.

#[cfg(target_arch = "wasm32")]
mod browser;

#[cfg(target_arch = "wasm32")]
fn main() -> std::io::Result<()> {
    browser::run()
}

/// Host build only serves `cargo test`/clippy: the dashboard needs a browser.
#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!("dbsrt-web-top runs in a browser: build it with `task web-top:build`.");
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    #[test]
    fn test_host_main_only_prints_build_hint() {
        super::main();
    }
}
