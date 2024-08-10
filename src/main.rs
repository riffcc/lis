use anyhow::Result;
use clap::Parser;

use lis::{Cli, Commands, Lis};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut lis = Lis::new(&cli.root, cli.overwrite).await?;

    match &cli.command {
        Commands::Add { paths } => {
            for path in paths {
                lis.add_file(path.as_path()).await?;
            }
        }
        Commands::List {} => {
            lis.list().await?;
        }
        &Commands::Get { .. } | &Commands::Rm { .. } => todo!(),
    }

    Ok(())
}
