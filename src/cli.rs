use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// If Iroh node is on disk set root dir (otherwise it's in memory)
    #[arg(short, long)]
    #[arg(long, value_parser)]
    disk: Option<PathBuf>,

    // /// Sets a custom config file
    // #[arg(short, long, value_name = "FILE")]
    // config: Option<PathBuf>,

    // /// Turn debugging information on
    // #[arg(short, long, action = clap::ArgAction::Count)]
    // debug: u8,
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Adds files or dirs to filesystem
    /// Paths that don't exist or aren't accessible are ignored
    Add {
        /// paths to add
        paths: Vec<PathBuf>,
    },
    /// Gets files or dirs from filesystem
    /// Paths that don't exist or aren't accessible are ignored
    Get {
        /// paths to get
        paths: Vec<PathBuf>,
    },
    /// Removes files or dirs to filesystem
    /// Paths that don't exist or aren't accessible are ignored
    Rm {
        /// paths to remove
        paths: Vec<PathBuf>,
    },
}
