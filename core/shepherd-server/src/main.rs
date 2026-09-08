//! shepherd daemon entry point — config parsing and serving. Routes live in
//! the lib target ([`shepherd_server::router`]); logic in `shepherd-core`.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use shepherd_core::Store;

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

    /// Path to the SQLite database file.
    #[arg(long, env = "SHEPHERD_DB")]
    db: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::parse();

    // Resolve database path: explicit flag, env var, or default.
    let db_path = config.db.unwrap_or_else(|| {
        let mut dir = dirs::home_dir().expect("cannot determine home directory");
        dir.push(".shepherd");
        std::fs::create_dir_all(&dir).expect("cannot create ~/.shepherd");
        dir.push("shepherd.db");
        dir
    });

    let store = Store::open(&db_path)
        .await
        .with_context(|| format!("failed to open database at {}", db_path.display()))?;

    let state = shepherd_server::AppState { store };
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, config.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind {addr}"))?;
    println!("shepherd-server listening on http://{addr}");
    axum::serve(listener, shepherd_server::router(state, &config.ui_dir))
        .await
        .context("server error")?;
    Ok(())
}
