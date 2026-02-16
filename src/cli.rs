use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};

pub const HANDSHAKE_PORT: u16 = 7878;
pub const TRANSFER_PORT: u16 = 7879;

#[derive(Parser)]
#[command(name = "sendfile")]
#[command(about = "Simple file transfer tool", long_about = None)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Send a file to a receiver
    Send(SendArgs),
    /// Receive a file and write it to a path
    Receive(ReceiveArgs),
}

#[derive(Args)]
pub struct SendArgs {
    /// Path to the file to send
    #[arg(name = "FILE")]
    pub file: PathBuf,

    /// Receiver host or IP
    #[arg(name = "HOST")]
    pub host: String,

    /// Block size in bytes
    #[arg(short, long)]
    pub block_size: Option<u32>,

    /// Number of concurrent connections [default: capped to min(os_threads, 16)]
    #[arg(short, long)]
    pub concurrency: Option<u16>,

    #[arg(long)]
    pub no_compress: bool,
}

#[derive(Args)]
pub struct ReceiveArgs {
    /// Output path. If a directory, place the incoming file inside it.
    /// If a file path, write to that exact path.
    #[arg(name = "PATH")]
    pub file: PathBuf,

    /// Number of concurrent connections [default: capped to min(os_threads, 16)]
    #[arg(short, long)]
    pub concurrency: Option<u16>,
}
