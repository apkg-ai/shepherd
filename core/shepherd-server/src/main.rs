//! shepherd daemon — deliberately thin: route definitions, request/response
//! mapping, SSE fan-out (from S6), and serving the built UI. Logic lives in
//! `shepherd-core`.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use anyhow::Context;
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use clap::Parser;
use tower_http::services::ServeDir;

/// The served contract. Embedded so the binary is self-contained.
const OPENAPI_SPEC: &str = include_str!("../../../openapi/shepherd.yaml");

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

fn router(config: &Config) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/v1/openapi.yaml", get(openapi_spec))
        .fallback_service(ServeDir::new(&config.ui_dir))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({
        "status": "ok",
        "version": shepherd_core::version(),
    }))
}

async fn openapi_spec() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "application/yaml")], OPENAPI_SPEC)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::parse();
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, config.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind {addr}"))?;
    println!("shepherd-server listening on http://{addr}");
    axum::serve(listener, router(&config))
        .await
        .context("server error")?;
    Ok(())
}
