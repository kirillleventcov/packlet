use clap::Parser;
use packlet::cli::{Cli, run};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    
    if cli.verbose {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    } else {
        env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    }

    if let Err(e) = run(cli).await {
        log::error!("Error: {}", e);
        std::process::exit(1);
    }
    
    Ok(())
}