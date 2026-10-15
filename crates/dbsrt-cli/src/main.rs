use clap::Parser;

#[derive(Parser, Debug)]
#[command(
    name = "dbsrt",
    version,
    about = "High-performance video perceptual hash & subtitle retargeting"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand, Debug)]
enum Commands {
    /// Benchmark pHash computation throughput
    BenchPhash {
        #[arg(short, long, default_value_t = 100_000)]
        count: usize,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Commands::BenchPhash { count } => {
            println!("Benchmarking pHash on {} frames...", count);
            let dummy_frame = [128u8; 1024];
            let start = std::time::Instant::now();
            let mut last_hash = 0;
            for _ in 0..count {
                last_hash ^= dbsrt_core::phash_32x32(&dummy_frame);
            }
            let elapsed = start.elapsed();
            let fps = count as f64 / elapsed.as_secs_f64();
            let per_frame_us = elapsed.as_secs_f64() * 1_000_000.0 / count as f64;
            println!("Result hash: {:016x}", last_hash);
            println!("Elapsed: {:?}", elapsed);
            println!(
                "Throughput: {:.0} frames/sec ({:.2} µs / frame)",
                fps, per_frame_us
            );
        }
    }
    Ok(())
}
