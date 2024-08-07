use anyhow::Result;
use futures_lite::StreamExt;
use lis::{Cli, Lis};
use std::path::Path;

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut lis = Lis::new(cli.disk).await?;

    lis.add_file(Path::new("/tmp/bigfile")).await?;

    for entry in lis.iroh_node.docs().list().await?.collect::<Vec<_>>().await {
        let (ns, cap) = entry?;
        println!("\t{ns}\t{cap}");
    }

    Ok(())
}
