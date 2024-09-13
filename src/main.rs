use anyhow::Result;
use clap::Parser;
#[allow(unused)]
use log::{debug, error, info, warn, LevelFilter};
use std::{
    io::Write,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use lis::{Cli, Commands, Lis, Manifest};

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
        Commands::Put { paths } => {
            for path in paths {
                info!(
                    "Added {} (keys: {:#?})",
                    path.display(),
                    lis.put(path.as_path(), Path::new(path.file_name().unwrap()))
                        .await?
                );
            }
        }
        Commands::Mkdir { path } => {
            info!(
                "Created {} (id: {:#?})",
                path.display(),
                lis.mkdir(path, None, None, None).await?
            );
        }
        Commands::List { path } => {
            let entries = match path {
                Some(path) => lis.list(path).await?,
                None => lis.list(Path::new("/")).await?,
            };
            for entry in entries {
                if let Ok(entry) = entry {
                    let key = entry.key();
                    let hash = entry.content_hash();
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

            debug!("Connecting...");
            std::io::stdout().flush()?;
            let conn = lis
                .iroh_node
                .endpoint()
                .connect(ticket.node_addr().clone(), iroh::blobs::protocol::ALPN)
                .await?;
            debug!("connected!");

            debug!("Receiving manifest...");
            std::io::stdout().flush()?;
            let mut stream = conn.accept_uni().await?;
            let manifest: Manifest = serde_json::from_slice(&stream.read_to_end(2048).await?)?;
            debug!("ok.");
            debug!("{:#?}", manifest);

            debug!("Saving manifest...");
            // TODO
            debug!("ok.");

            debug!("All done.");
        }
        Commands::Invite {} => {
            let ticket = lis.invite().await?;
            let endpoint = lis.iroh_node.endpoint().clone();
            // TODO: timeout
            let handle = tokio::spawn(async move {
                if let Some(conn) = endpoint.accept().await {
                    debug!("Connecting with {}", conn.remote_address());
                    let conn = conn.await.unwrap();

                    debug!("Updating manifest...");
                    // TODO update manifest
                    debug!("ok.");

                    debug!("Sending manifest...");
                    let serialized_manifest = serde_json::to_string(&lis.manifest).unwrap();
                    let mut stream = conn.open_uni().await.unwrap();
                    stream
                        .write_all(&serialized_manifest.as_bytes())
                        .await
                        .unwrap();
                    stream.finish().await.unwrap();
                    debug!("ok.");

                    debug!("All done.");
                }
            });

            println!("\n\n\tlis <lis_root> join {ticket}\n");
            handle.await?;
        }
        Commands::Mount { mountpoint } => {
            let _handle = fuser::spawn_mount2(lis, &mountpoint, &[])?;
            let stop = Arc::new(AtomicBool::new(false));

            let stop_clone = stop.clone();
            let mountpoint_clone = mountpoint.clone();
            ctrlc::set_handler(move || {
                println!("unmounting {}", mountpoint_clone.display());
                stop_clone.store(true, Ordering::SeqCst);
            })?;
            while !stop.load(Ordering::SeqCst) {}
        }
    }

    Ok(())
}
