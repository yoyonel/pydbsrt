//! Subtitle search, candidate ranking, CAS download, and visual fingerprint recognition handlers.

pub mod cas;
pub mod download;
pub mod lookup;
pub mod recognize;

pub use cas::{get_subtitle_by_hash_handler, scan_handler};
pub use download::download_handler;
pub use lookup::search_handler;
pub use recognize::recognize_handler;
