use anyhow::{Result, ensure};
use clap::Parser;
use std::{net::SocketAddr, path::PathBuf};

#[derive(Parser)]
#[command(about = "Croaker local UI and multi-account test workspace. Never deploy to AWS.")]
struct Args {
    #[arg(long, default_value = "127.0.0.1:3000")]
    bind: SocketAddr,
    #[arg(long, default_value = "data/ui")]
    data_dir: PathBuf,
    #[arg(long)]
    assets: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    ensure!(
        args.bind.ip().is_loopback(),
        "The test workspace can only bind to loopback."
    );
    let assets = args
        .assets
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/ui/dist"));
    ensure!(
        assets.join("index.html").is_file(),
        "Build the UI first with npm run build:ui, or start everything with npm run dev."
    );
    let listener = tokio::net::TcpListener::bind(args.bind).await?;
    let address = listener.local_addr()?;
    let workspace = croaker_local::Workspace::open(args.data_dir).await?;
    println!(
        "\n  Croaker is ready → http://{address}\n  Open /?as=bob in a second tab to test messaging.\n  Local test accounts only. Press Ctrl+C to stop.\n"
    );
    croaker_relay::serve(
        listener,
        croaker_local::router(workspace, assets, address.port()),
    )
    .await?;
    Ok(())
}
