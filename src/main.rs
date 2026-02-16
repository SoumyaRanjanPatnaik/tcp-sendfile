use clap::Parser;
use log::{error, info};
use sendfile::cli::{Cli, Commands, HANDSHAKE_PORT};
use sendfile::stream;
use sendfile::transport::MAX_BLOCK_SIZE;

fn get_concurrency(requested: Option<u16>) -> u16 {
    let available = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(1);
    let max_concurrency = available.min(16);
    let concurrency = requested.unwrap_or(max_concurrency as u16);

    // Cap concurrency if user provides a higher value
    let effective = concurrency.min(max_concurrency as u16);
    if effective < concurrency {
        info!("Capped concurrency from {} to {}", concurrency, effective);
    }
    effective
}

fn main() {
    env_logger::init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Send(args) => {
            let address = (args.host.as_str(), HANDSHAKE_PORT);
            let block_size = args.block_size.unwrap_or(1024 * 1024).min(MAX_BLOCK_SIZE); // Default 1MB

            info!(
                "Sending file {:?} to {}:{} (block_size: {})",
                args.file, address.0, address.1, block_size
            );

            if let Err(e) = stream::send::send_file(address, &args.file, block_size) {
                error!("Failed to send file: {}", e);
                std::process::exit(1);
            }
        }
        Commands::Receive(args) => {
            let concurrency = get_concurrency(args.concurrency);
            let bind_address = ("0.0.0.0", HANDSHAKE_PORT);

            info!(
                "Receiving file at {}:{} (output: {:?}, concurrency: {})",
                bind_address.0, bind_address.1, args.file, concurrency
            );

            if let Err(e) = stream::receive::receive_file(bind_address, &args.file, concurrency) {
                error!("Failed to receive file: {}", e);
                std::process::exit(1);
            }
        }
    }
}
