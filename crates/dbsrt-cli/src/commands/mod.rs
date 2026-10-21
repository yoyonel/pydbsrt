//! Modular CLI command handlers implementing Separation of Concerns (SoC).

pub mod bench;
pub mod certify;
pub mod export;
pub mod fetch;
pub mod import;
pub mod inspect;
pub mod pack;
pub mod recognize;
pub mod retarget;
pub mod scan;

pub use bench::{run_bench_cuts, run_bench_phash, run_bench_video, BenchCutsArgs, BenchVideoArgs};
pub use certify::{detect_cli_subtitle_lang, run_certify_coupling, CertifyCouplingArgs};
pub use export::{run_export_phash, ExportPhashArgs};
pub use fetch::{run_fetch_subtitles, FetchSubtitlesArgs};
pub use import::{run_import_phash, ImportPhashArgs};
pub use inspect::{run_inspect_media, InspectMediaArgs};
pub use pack::{extract_fps_from_path, run_pack, PackArgs};
pub use recognize::{format_set, run_recognize_media, AggregatedMatch, RecognizeArgs};
pub use retarget::{find_or_build_phash, run_retarget_subtitles, RetargetArgs};
pub use scan::{run_scan_library, ScanLibraryArgs};
