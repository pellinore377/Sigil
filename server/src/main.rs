//! sigil-server: the home server and the Envoy, one binary.

mod config;
mod delivery;
mod envoy;
mod home;
mod http;
mod store;
mod tokens;

use clap::{Parser, Subcommand};
use config::Config;
use std::path::PathBuf;
use std::sync::Arc;

#[derive(Parser)]
#[command(
    name = "sigil-server",
    about = "The Sigil home server and Envoy in one binary"
)]
struct Cli {
    /// Path to the TOML config.
    #[arg(short, long, default_value = "sigil.toml")]
    config: PathBuf,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Write a default config and create the data directory.
    Init {
        #[arg(long)]
        hostname: String,
        #[arg(long, default_value = "both")]
        role: String,
        #[arg(long, default_value = "0.0.0.0:8443")]
        listen: String,
    },
    /// Run.
    Run,
    /// Create an invite code for registration.
    Invite,
    /// Print this server's card as hex.
    Card,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("sigil_server=info".parse()?),
        )
        .init();
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Init {
            hostname,
            role,
            listen,
        } => {
            let cfg = Config {
                hostname,
                role,
                listen,
                ..Default::default()
            };
            std::fs::write(&cli.config, toml::to_string_pretty(&cfg)?)?;
            store::Store::open(&cfg.data_dir)?;
            let token_path = cfg.data_dir.join("admin.token");
            if !token_path.exists() {
                std::fs::write(&token_path, hex::encode(rand::random::<[u8; 16]>()))?;
            }
            println!(
                "wrote {} and created {}",
                cli.config.display(),
                cfg.data_dir.display()
            );
        }
        Cmd::Invite => {
            let cfg = Config::load(&cli.config)?;
            // Ask the running server over loopback; fall back to the database
            // directly when the server is not running.
            let token =
                std::fs::read_to_string(cfg.data_dir.join("admin.token")).unwrap_or_default();
            let scheme = if cfg.tls_cert.is_some() {
                "https"
            } else {
                "http"
            };
            let port = cfg.listen.rsplit(':').next().unwrap_or("8443");
            let url = format!("{scheme}://127.0.0.1:{port}/admin/invite");
            let client = reqwest::Client::builder()
                .danger_accept_invalid_certs(true)
                .build()?;
            match client
                .post(&url)
                .header("x-sigil-admin", token.trim())
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => println!("{}", r.text().await?),
                Ok(r) => anyhow::bail!("server refused: {}", r.status()),
                Err(_) => {
                    let st = store::Store::open(&cfg.data_dir)?;
                    let code = hex::encode(rand::random::<[u8; 8]>());
                    let w = st.db.begin_write()?;
                    w.open_table(store::INVITES)?.insert(code.as_str(), ())?;
                    w.commit()?;
                    println!("{code}");
                }
            }
        }
        Cmd::Card => {
            let cfg = Config::load(&cli.config)?;
            let st = Arc::new(store::Store::open(&cfg.data_dir)?);
            let home = home::Home::new(cfg, st)?;
            println!("{}", hex::encode(&home.card));
        }
        Cmd::Run => run(Config::load(&cli.config)?).await?,
    }
    Ok(())
}

async fn run(cfg: Config) -> anyhow::Result<()> {
    let store = Arc::new(store::Store::open(&cfg.data_dir)?);
    let home = if cfg.is_home() {
        Some(home::Home::new(cfg.clone(), store.clone())?)
    } else {
        None
    };
    let envoy = if cfg.is_envoy() {
        Some(envoy::Envoy::new(cfg.clone(), store.clone(), home.clone())?)
    } else {
        None
    };
    if let (Some(e), Some(h)) = (&envoy, &home) {
        // open the in-process delivery stream to ourselves
        let _ = e.ensure_stream(&h.cfg.hostname).await;
    }
    let app = http::router(http::App { home, envoy });
    let addr: std::net::SocketAddr = cfg.listen.parse()?;
    match (&cfg.tls_cert, &cfg.tls_key) {
        (Some(c), Some(k)) => {
            let tls = axum_server::tls_rustls::RustlsConfig::from_pem_file(c, k).await?;
            tracing::info!("sigil-server ({}) listening on https://{addr}", cfg.role);
            axum_server::bind_rustls(addr, tls)
                .serve(app.into_make_service())
                .await?;
        }
        _ => {
            tracing::warn!("no TLS configured: plain HTTP is for local testing only");
            tracing::info!("sigil-server ({}) listening on http://{addr}", cfg.role);
            axum_server::bind(addr)
                .serve(app.into_make_service())
                .await?;
        }
    }
    Ok(())
}
