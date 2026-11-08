use anyhow::{Result, ensure};
use clap::Parser;
use std::{net::SocketAddr, path::PathBuf};

#[derive(Parser)]
#[command(about = "Cirra local UI and multi-account test workspace. Never deploy to AWS.")]
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
    let options = croaker_local::auth::AuthOptions::from_env(address.port())?;
    let mut workspace = croaker_local::Workspace::open(args.data_dir).await?;
    workspace.configure_login(&options).await?;
    println!(
        "\n  Cirra is ready → http://{address}\n  Sign in with a test account. Use another browser profile to chat.\n  Local test accounts only. Press Ctrl+C to stop.\n"
    );
    croaker_relay::serve(
        listener,
        croaker_local::router_with_auth(workspace, assets, address.port(), options),
    )
    .await?;
    Ok(())
}
