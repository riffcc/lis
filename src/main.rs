use anyhow::Result;
use clap::Parser;
use std::io::Write;

use lis::{Cli, Commands, Lis, Manifest};

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let mut lis = Lis::new(&cli.root, cli.overwrite).await?;

    match &cli.command {
        Commands::Put { paths } => {
            for path in paths {
                println!(
                    "Added {} (keys: {:#?})",
                    path.display(),
                    lis.put(path.as_path()).await?
                );
            }
        }
        Commands::List {} => {
            let entries = lis.list().await?;
            for entry in entries {
                if let Ok(entry) = entry {
                    let key = entry.key();
                    let hash = entry.content_hash();
                    // let author = entry.author();
                    // let content = entry.content_bytes(self.iroh_node.client()).await?;
                    println!("{} ({})", std::str::from_utf8(key)?, hash.fmt_short());
                }
            }
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
        Commands::Join { ticket } => {
            lis.join(ticket)?;

            print!("Connecting...");
            std::io::stdout().flush()?;
            let conn = lis
                .iroh_node
                .endpoint()
                .connect(ticket.node_addr().clone(), iroh::blobs::protocol::ALPN)
                .await?;
            println!("connected!");

            print!("Receiving manifest...");
            std::io::stdout().flush()?;
            let mut stream = conn.accept_uni().await?;
            let manifest: Manifest = serde_json::from_slice(&stream.read_to_end(2048).await?)?;
            println!("ok.");
            println!("{:#?}", manifest);

            print!("Saving manifest...");
            // TODO
            println!("ok.");

            println!("All done.");
        }
        Commands::Invite {} => {
            let ticket = lis.invite().await?;
            let endpoint = lis.iroh_node.endpoint().clone();
            // TODO: timeout
            let handle = tokio::spawn(async move {
                if let Some(conn) = endpoint.accept().await {
                    println!("Connecting with {}", conn.remote_address());
                    let conn = conn.await.unwrap();

                    print!("Updating manifest...");
                    // TODO update manifest
                    println!("ok.");

                    print!("Sending manifest...");
                    let serialized_manifest = serde_json::to_string(&lis.manifest).unwrap();
                    let mut stream = conn.open_uni().await.unwrap();
                    stream
                        .write_all(&serialized_manifest.as_bytes())
                        .await
                        .unwrap();
                    stream.finish().await.unwrap();
                    println!("ok.");

                    println!("All done.");
                }
            });

            println!("\n\n\tlis --root <some_root> join {ticket}\n");
            handle.await?;
        }
        Commands::Mount { mountpoint } => {
            fuser::mount2(lis, &mountpoint, &[])?;
        }
    }

    Ok(())
}
