//! Axum HTTP handler endpoints.

pub mod autosync;
pub mod health;
pub mod search;
pub mod studio;

pub use autosync::*;
pub use health::*;
pub use search::*;
pub use studio::*;
