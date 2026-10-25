//! Modular CLI command handlers implementing Separation of Concerns (SoC).

pub mod bench;
pub mod certify;
pub mod export;
pub mod fetch;
pub mod import;
pub mod inspect;
pub mod media_add;
pub mod migrate;
pub mod pack;
pub mod recognize;
pub mod relabel;
pub mod retarget;
pub mod scan;
pub mod shared;

pub use bench::{run_bench_cuts, run_bench_phash, run_bench_video, BenchCutsArgs, BenchVideoArgs};
#[allow(deprecated)]
pub use certify::{
    detect_cli_subtitle_lang, detect_cli_subtitle_lang_status, run_certify_coupling,
    CertifyCouplingArgs,
};
pub use export::{run_export_phash, ExportPhashArgs};
pub use fetch::{run_fetch_subtitles, FetchSubtitlesArgs};
pub use import::{run_import_phash, ImportPhashArgs};
pub use inspect::{run_inspect_media, InspectMediaArgs};
pub use media_add::{run_media_add, MediaAddArgs, MediaAddResult};
pub use migrate::{run_migrate, MigrateArgs};
pub use pack::{extract_fps_from_path, run_pack, PackArgs};
pub use recognize::{format_set, run_recognize_media, AggregatedMatch, RecognizeArgs};
pub use relabel::{run_relabel_coupling, RelabelCouplingArgs};
pub use retarget::{run_retarget_subtitles, RetargetArgs};
pub use scan::{run_scan_library, ScanLibraryArgs};
pub use shared::{connect_db, find_or_build_phash, resolve_db_host};

#[cfg(test)]
pub(crate) static LIVE_DB_TEST_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());
