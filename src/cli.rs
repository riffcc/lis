use std::path::PathBuf;

use clap::{ArgAction, Parser, Subcommand};
use iroh::net::ticket::NodeTicket;

#[derive(Parser)]
#[command(version, about, long_about = None)]
pub struct Cli {
    /// If Iroh node is on disk set root dir (otherwise it's in memory)
    #[arg(long, value_parser)]
    pub root: PathBuf,

    #[arg(short, long)]
    pub overwrite: bool,

    /// Verbose mode (-v, -vv, -vvv, -vvvv)
    #[arg(short, long, action = ArgAction::Count)]
    pub verbosity: u8,

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
    /// Puts files into node
    /// Paths that don't exist or aren't accessible are ignored
    Put { paths: Vec<PathBuf> },
    /// List files on filesystem
    /// Paths that don't exist or aren't accessible are ignored
    #[command(alias = "ls")]
    List {},
    /// Gets files that are not currently locally accessible
    /// Paths that don't exist or aren't accessible are ignored
    Get { paths: Vec<PathBuf> },
    /// Removes files or dirs to filesystem
    /// Paths that don't exist or aren't accessible are ignored
    Rm { paths: Vec<PathBuf> },
    /// Joins a network using the given ticket
    Join { ticket: NodeTicket },
    /// Generates a ticket for joining a network with Join
    Invite {},
    /// Mounts path and keeps mounted while cli is running
    Mount { mountpoint: PathBuf },
}
