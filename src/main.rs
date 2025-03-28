use std::path::Path;

use anyhow::Result;
use clap::Parser;
#[allow(unused)]
use log::{debug, error, info, warn, LevelFilter};

use lis::{Cli, Commands, Lis};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let log_level = match cli.verbosity {
        0 => LevelFilter::Error,
        1 => LevelFilter::Warn,
        2 => LevelFilter::Info,
        3 => LevelFilter::Debug,
        _ => LevelFilter::Trace,
    };

    let mut log_builder = env_logger::builder();
    log_builder.format_timestamp_nanos();
    log_builder.filter(Some("lis"), log_level);
    log_builder.filter(None, log::LevelFilter::Off);
    log_builder.init();

    let mut lis = Lis::new(&cli.root, cli.overwrite).await?;

    match &cli.command {
        Commands::Touch { path } => _ = lis.create_file(&path).await?,
        Commands::Mkdir { path } => lis.create_dir(&path).await?,
        Commands::List { path } => {
            let paths = lis
                .list(&path.clone().unwrap_or(Path::new("/").to_path_buf()))
                .await?;
            for path in paths {
                println!("{}", path.display());
            }
        }
        Commands::ImportFile { paths: _ } => {}
        Commands::Read { paths: _ } => {}
        Commands::Rm { paths: _ } => {}
        Commands::Rmdir { paths: _ } => {}
        Commands::Join { ticket: _ } => {}
        Commands::Invite {} => {}
        Commands::Mount { mountpoint: _ } => {}
    }

    Ok(())
}
