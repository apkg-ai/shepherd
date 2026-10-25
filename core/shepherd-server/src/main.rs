use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use shepherd_core::commands::IdentityPaths;
use shepherd_core::model::SystemClock;
use shepherd_core::storage::{
    AesGcmCodec, DaemonLock, FileReplayKeyProvider, StoreOptions, maintenance, open,
};
use shepherd_server::{AppState, AuthConfig};

/// Local-first hub for agent-driven work.
#[derive(Parser, Debug)]
#[command(name = "shepherd-server", version)]
struct Cli {
    /// Data directory for the database and credential files (plan/13).
    #[arg(long, env = "SHEPHERD_DATA_DIR", global = true)]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Start the HTTP daemon.
    Serve {
        /// Port to bind on 127.0.0.1.
        #[arg(long, env = "SHEPHERD_PORT", default_value_t = 7437)]
        port: u16,

        /// Directory of built UI assets served at `/`.
        #[arg(long, env = "SHEPHERD_UI_DIR", default_value = "ui/dist")]
        ui_dir: PathBuf,

        /// Allow the Vite dev origin http://localhost:5173.
        #[arg(long)]
        dev: bool,

        /// Rotate the owner credential and rewrite <data-dir>/owner-token.
        #[arg(long)]
        reissue_owner_token: bool,
    },
    /// Create an offline backup of the data directory.
    Backup {
        /// Directory to write the backup into (must not already exist).
        #[arg(long)]
        output: PathBuf,
    },
    /// Restore from an offline backup.
    Restore {
        /// Directory containing the backup to restore from.
        #[arg(long)]
        from: PathBuf,
    },
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

fn resolve_data_dir(explicit: &Option<PathBuf>) -> anyhow::Result<PathBuf> {
    let data_dir = match explicit {
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
    Ok(data_dir)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let data_dir = resolve_data_dir(&cli.data_dir)?;

    match cli.command {
        Command::Serve {
            port,
            ui_dir,
            dev,
            reissue_owner_token,
        } => run_serve(data_dir, port, ui_dir, dev, reissue_owner_token).await,
        Command::Backup { output } => run_backup(data_dir, output).await,
        Command::Restore { from } => run_restore(data_dir, from).await,
    }
}

async fn run_serve(
    data_dir: PathBuf,
    port: u16,
    ui_dir: PathBuf,
    dev: bool,
    reissue_owner_token: bool,
) -> anyhow::Result<()> {
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
        None if reissue_owner_token => {
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
            if reissue_owner_token {
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

    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .with_context(|| format!("failed to bind {addr}"))?;
    let local_addr = listener.local_addr().context("failed to read bound addr")?;
    let auth = AuthConfig::new(local_addr.port(), dev);
    let state = match bootstrapped {
        Bootstrapped::Normal(store) => AppState::new(store, auth),
        Bootstrapped::Diagnostic(reason) => AppState::diagnostic(reason, auth),
    };
    // Resolved nonsecret paths only (plan/13); token values never print.
    println!("shepherd-server listening on http://{local_addr}");
    println!("data dir: {}", data_dir.display());
    println!("database: {}", db_path.display());
    println!("ui dir: {}", ui_dir.display());
    axum::serve(listener, shepherd_server::router(state, &ui_dir))
        .await
        .context("server error")?;
    Ok(())
}

async fn run_backup(data_dir: PathBuf, output: PathBuf) -> anyhow::Result<()> {
    maintenance::backup(&data_dir, &output)
        .await
        .with_context(|| format!("backup to {} failed", output.display()))?;
    println!("backup created at {}", output.display());
    Ok(())
}

async fn run_restore(data_dir: PathBuf, from: PathBuf) -> anyhow::Result<()> {
    let recovery = maintenance::restore(&data_dir, &from)
        .await
        .with_context(|| format!("restore from {} failed", from.display()))?;
    println!(
        "restore complete; previous data preserved at {}",
        data_dir.join(&recovery).display()
    );
    Ok(())
}
