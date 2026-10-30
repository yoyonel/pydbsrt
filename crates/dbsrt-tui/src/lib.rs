//! `dbsrt-tui`: Platform-agnostic terminal user interface components, views, and data models
//! for pydbsrt process monitoring and catalog inspection.
//!
//! Compiles both natively and to WebAssembly (`wasm32-unknown-unknown`) with zero OS/libc dependencies.

pub mod app;
pub mod constants;
pub mod host_info;
pub mod model;
pub mod ui;
pub mod views;

pub use constants::*;
pub use host_info::*;
pub use model::*;
pub use ui::*;
pub use views::*;
