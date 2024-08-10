use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// If Iroh node is on disk set root dir (otherwise it's in memory)
    #[arg(long, value_parser)]
    pub root: PathBuf,

    #[arg(short, long)]
    pub overwrite: bool,

    // /// Sets a custom config file
    // #[arg(short, long, value_name = "FILE")]
    // config: Option<PathBuf>,

    // /// Turn debugging information on
    // #[arg(short, long, action = clap::ArgAction::Count)]
    // debug: u8,
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Adds files
    /// Paths that don't exist or aren't accessible are ignored
    Add {
        /// paths to add
        paths: Vec<PathBuf>,
    },
    /// List files on filesystem
    /// Paths that don't exist or aren't accessible are ignored
    #[command(alias = "ls")]
    List {},
    /// Gets files that are not currently locally accessible
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
