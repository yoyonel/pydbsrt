use anyhow::Result;
use clap::Parser;
use dbsrt_cli::cli::{Cli, Commands};
use dbsrt_cli::commands::{
    run_bench_cuts, run_bench_phash, run_bench_video, run_certify_coupling, run_export_ahash,
    run_export_phash, run_fetch_subtitles, run_import_phash, run_inspect_media,
    run_match_trajectory, run_media_add, run_migrate, run_pack, run_recognize_media,
    run_relabel_coupling, run_retarget_subtitles, run_scan_library, run_top,
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

fn dispatch_eval_and_media(cmd: Commands) -> Result<Option<Commands>> {
    match cmd {
        Commands::BenchPhash { count } => {
            run_bench_phash(count)?;
            Ok(None)
        }
        Commands::BenchVideo(args) => {
            run_bench_video(&args)?;
            Ok(None)
        }
        Commands::RecognizeMedia(args) => {
            run_recognize_media(&args)?;
            Ok(None)
        }
        Commands::MatchTrajectory(args) => {
            run_match_trajectory(args)?;
            Ok(None)
        }
        Commands::RetargetSubtitles(args) => {
            run_retarget_subtitles(&args)?;
            Ok(None)
        }
        Commands::ExportPhash(args) => {
            run_export_phash(&args)?;
            Ok(None)
        }
        Commands::ExportAhash(args) => {
            run_export_ahash(&args)?;
            Ok(None)
        }
        Commands::ScanLibrary(args) => {
            run_scan_library(&args)?;
            Ok(None)
        }
        Commands::BenchCuts(args) => {
            run_bench_cuts(&args)?;
            Ok(None)
        }
        other => Ok(Some(other)),
    }
}

fn dispatch_ops(cmd: Commands) -> Result<()> {
    match cmd {
        Commands::ImportPhash(args) => run_import_phash(&args)?,
        Commands::CertifyCoupling(args) => run_certify_coupling(&args)?,
        Commands::InspectMedia(args) => run_inspect_media(&args)?,
        Commands::Pack(args) => run_pack(&args)?,
        Commands::FetchSubtitles(args) => run_single_thread(run_fetch_subtitles(&args))?,
        Commands::Migrate(args) => run_migrate(args)?,
        Commands::MediaAdd(args) => drop(run_single_thread(run_media_add(&args))?),
        Commands::RelabelCoupling(args) => run_relabel_coupling(args)?,
        Commands::Top(args) => run_top(&args)?,
        _ => unreachable!(),
    }
    Ok(())
}

fn main() -> Result<()> {
    // Constrain Rayon thread pool to prevent NPROC exhaustion on shared hosting (CloudLinux LVE)
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(dbsrt_cli::commands::top::constants::DEFAULT_MAX_RAYON_WORKER_THREADS)
        .build_global();

    let cli = Cli::parse();
    if let Some(remaining) = dispatch_eval_and_media(cli.command)? {
        dispatch_ops(remaining)?;
    }
    Ok(())
}
