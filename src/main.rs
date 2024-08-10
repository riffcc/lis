use anyhow::Result;
use clap::Parser;

use lis::{Cli, Commands, Lis};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut lis = Lis::new(&cli.root, cli.overwrite).await?;

    match &cli.command {
        Commands::Put { paths } => {
            for path in paths {
                if path.is_file() {
                    println!(
                        "Added {} (key: {})",
                        path.display(),
                        lis.put_file(path.as_path()).await?
                    );
                } else if path.is_dir() {
                    todo!()
                } // TODO: implement
            }
        }
        Commands::List {} => {
            lis.list().await?;
        }
        Commands::Get { paths } => {
            for path in paths {
                let content = lis.get_file(path.as_path()).await?;
                // Convert to &str
                let ascii_content = std::str::from_utf8(&content)?;
                println!("{}\n\n{}", path.display(), ascii_content);
            }
        }
        Commands::Rm { paths } => {
            for path in paths {
                let key = lis.rm_file(path.as_path()).await?;
                println!("Removed {key}");
            }
        }
    }

    Ok(())
}
