use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::Parser;
use shepherd_core::commands::IdentityPaths;
use shepherd_core::model::SystemClock;
use shepherd_core::storage::{AesGcmCodec, DaemonLock, FileReplayKeyProvider, StoreOptions, open};
use shepherd_server::{AppState, AuthConfig};

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

    /// Data directory for the database and credential files (plan/13).
    #[arg(long, env = "SHEPHERD_DATA_DIR")]
    data_dir: Option<PathBuf>,

    /// Allow the Vite dev origin http://localhost:5173.
    #[arg(long)]
    dev: bool,

    /// Rotate the owner credential and rewrite <data-dir>/owner-token.
    #[arg(long)]
    reissue_owner_token: bool,
}

fn default_data_dir() -> anyhow::Result<PathBuf> {
    let home = std::env::var_os("HOME").context("HOME is not set; pass --data-dir")?;
    let home = PathBuf::from(home);
    if cfg!(target_os = "macos") {
        Ok(home.join("Library/Application Support/Shepherd/v1"))
    } else {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| home.join(".local/share"));
        Ok(base.join("shepherd/v1"))
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let config = Config::parse();

    let data_dir = match &config.data_dir {
        Some(dir) => dir.clone(),
        None => default_data_dir()?,
    };
    std::fs::create_dir_all(&data_dir)
        .with_context(|| format!("failed to create data dir {}", data_dir.display()))?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&data_dir, std::fs::Permissions::from_mode(0o700))
            .with_context(|| format!("failed to restrict data dir {}", data_dir.display()))?;
    }
    let paths = IdentityPaths::new(&data_dir);

    // One daemon per data directory (plan/05 advisory lock).
    let _lock = DaemonLock::acquire(&paths.daemon_lock())
        .with_context(|| "another shepherd-server owns this data directory")?;

    let db_path = data_dir.join("shepherd.db");
    let replay_key_path = paths.replay_key();
    // Never silently regenerate a lost replay key (plan/12): existing installs
    // without one run diagnostic-only.
    let provider = if replay_key_path.exists() {
        Some(FileReplayKeyProvider::load(&replay_key_path).context("replay key")?)
    } else if db_path.exists() {
        None
    } else {
        Some(FileReplayKeyProvider::provision(&replay_key_path).context("replay key")?)
    };

    enum Bootstrapped {
        Normal(Arc<shepherd_core::storage::Store>),
        Diagnostic(String),
    }
    let bootstrapped = match provider {
        // Rotation needs the codec's key; silently ignoring the flag here would
        // let the owner believe a compromised credential was replaced (plan/12).
        None if config.reissue_owner_token => {
            return Err(anyhow::anyhow!(
                "cannot reissue the owner token: the replay key {} is missing; \
                 restore it from backup first",
                replay_key_path.display()
            ));
        }
        None => Bootstrapped::Diagnostic(format!(
            "replay key {} is missing; restore it from backup to leave \
             diagnostic-only mode",
            replay_key_path.display()
        )),
        Some(provider) => {
            let store = open(StoreOptions {
                db_path: db_path.clone(),
                mvp_db_path: None,
                clock: Arc::new(SystemClock),
                codec: Arc::new(AesGcmCodec::new(Arc::new(provider))),
            })
            .await
            .with_context(|| format!("failed to open {}", db_path.display()))?;
            let store = Arc::new(store);
            if config.reissue_owner_token {
                store
                    .reissue_owner_token(&paths)
                    .await
                    .context("failed to reissue the owner token")?;
                println!("owner token reissued at {}", paths.owner_token().display());
            }
            // Fail-loud bootstrap: a missing or mismatched owner-token file
            // aborts startup naming --reissue-owner-token.
            store
                .ensure_owner(&paths)
                .await
                .context("owner bootstrap failed")?;
            Bootstrapped::Normal(store)
        }
    };

    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, config.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind {addr}"))?;
    let local_addr = listener.local_addr().context("failed to read bound addr")?;
    // The Host/Origin allowlist uses the actual bound port (`--port 0` picks one).
    let auth = AuthConfig {
        port: local_addr.port(),
        dev: config.dev,
    };
    let state = match bootstrapped {
        Bootstrapped::Normal(store) => AppState::new(store, auth),
        Bootstrapped::Diagnostic(reason) => AppState::diagnostic(reason, auth),
    };
    // Resolved nonsecret paths only (plan/13); token values never print.
    println!("shepherd-server listening on http://{local_addr}");
    println!("data dir: {}", data_dir.display());
    println!("database: {}", db_path.display());
    println!("ui dir: {}", config.ui_dir.display());
    axum::serve(listener, shepherd_server::router(state, &config.ui_dir))
        .await
        .context("server error")?;
    Ok(())
}
