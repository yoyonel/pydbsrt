//! Axum HTTP handler endpoints.

pub mod autosync;
pub mod catalog_handlers;
pub mod health;
pub mod monitoring;
pub mod search;
pub mod studio;

pub use autosync::*;
pub use catalog_handlers::*;
pub use health::*;
pub use monitoring::*;
pub use search::*;
pub use studio::*;
