mod ai;
mod collect;
mod config;
mod tui;

use anyhow::Result;
use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "headroom", about = "System stability monitor — memory, paging, commit pressure")]
struct Args {
    /// Print a one-shot JSON snapshot and exit (no TUI)
    #[arg(long)]
    snapshot: bool,

    /// With --snapshot: also send to Claude for AI recommendations
    #[arg(long, requires = "snapshot")]
    ai: bool,

    /// TUI refresh interval in seconds
    #[arg(long, default_value = "2")]
    interval: u64,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    let args = Args::parse();
    let cfg = config::Config::load()?;

    if args.snapshot {
        let snap = collect::collect_snapshot()?;
        if args.ai {
            println!("Analyzing...\n");
            let recs = ai::get_recommendations(&snap, &cfg).await?;
            println!("{recs}");
        } else {
            println!("{}", serde_json::to_string_pretty(&snap)?);
        }
        return Ok(());
    }

    tui::run(cfg, args.interval).await
}
