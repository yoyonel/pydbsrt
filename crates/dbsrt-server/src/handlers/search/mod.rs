//! Subtitle search, candidate ranking, CAS download, and visual fingerprint recognition handlers.

pub mod ahash;
pub mod cas;
pub mod download;
pub mod lookup;
pub mod recognize;
pub mod telemetry;

pub use ahash::search_ahash_handler;
pub use cas::{get_subtitle_by_hash_handler, scan_handler};
pub use download::download_handler;
pub use lookup::search_handler;
pub use recognize::recognize_handler;
pub use telemetry::{latest_search_handler, list_snippets_handler};
