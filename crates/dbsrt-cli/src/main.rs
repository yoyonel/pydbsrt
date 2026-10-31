use anyhow::Result;
use clap::Parser;
use dbsrt_cli::cli::{Cli, Commands};
use dbsrt_cli::commands::{
    run_bench_cuts, run_bench_phash, run_bench_video, run_certify_coupling, run_export_phash,
    run_fetch_subtitles, run_import_phash, run_inspect_media, run_media_add, run_migrate, run_pack,
    run_recognize_media, run_relabel_coupling, run_retarget_subtitles, run_scan_library, run_top,
};

fn run_single_thread<F, R>(future: F) -> Result<R>
where
    F: std::future::Future<Output = Result<R>>,
{
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(future)
}

fn main() -> Result<()> {
    // Constrain Rayon thread pool to prevent NPROC exhaustion on shared hosting (CloudLinux LVE)
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(dbsrt_cli::commands::top::constants::DEFAULT_MAX_RAYON_WORKER_THREADS)
        .build_global();

    let cli = Cli::parse();
    match cli.command {
        Commands::BenchPhash { count } => run_bench_phash(count)?,
        Commands::BenchVideo(args) => run_bench_video(&args)?,
        Commands::RecognizeMedia(args) => run_recognize_media(&args)?,
        Commands::RetargetSubtitles(args) => run_retarget_subtitles(&args)?,
        Commands::ExportPhash(args) => run_export_phash(&args)?,
        Commands::ScanLibrary(args) => run_scan_library(&args)?,
        Commands::BenchCuts(args) => run_bench_cuts(&args)?,
        Commands::ImportPhash(args) => run_import_phash(&args)?,
        Commands::CertifyCoupling(args) => run_certify_coupling(&args)?,
        Commands::InspectMedia(args) => run_inspect_media(&args)?,
        Commands::Pack(args) => run_pack(&args)?,
        Commands::FetchSubtitles(args) => run_single_thread(run_fetch_subtitles(&args))?,
        Commands::Migrate(args) => run_migrate(args)?,
        Commands::MediaAdd(args) => drop(run_single_thread(run_media_add(&args))?),
        Commands::RelabelCoupling(args) => run_relabel_coupling(args)?,
        Commands::Top(args) => run_top(&args)?,
    }
    Ok(())
}
