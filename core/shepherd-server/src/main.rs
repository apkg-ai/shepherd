//! shepherd daemon entry point — config parsing and serving. Routes live in
//! the lib target ([`shepherd_server::router`]); logic in `shepherd-core`.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;

/// Local-first hub for agent-driven work.
#[derive(Parser, Debug)]
#[command(name = "shepherd-server", version)]
struct Config {
    /// Port to bind on 127.0.0.1.
    #[arg(long, env = "SHEPHERD_PORT", default_value_t = 7437)]
    port: u16,

    /// Directory of built UI assets served at `/`.
    #[arg(long, env = "SHEPHERD_UI_DIR", default_value = "ui/dist")]
    ui_dir: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::parse();
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, config.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind {addr}"))?;
    println!("shepherd-server listening on http://{addr}");
    axum::serve(listener, shepherd_server::router(&config.ui_dir))
        .await
        .context("server error")?;
    Ok(())
}
