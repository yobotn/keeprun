//! Keeprun agent.
//!
//! `keeprun-agent run`        Run in the foreground (development, debugging).
//! `keeprun-agent install`    Install and start the Windows service (needs Administrator).
//! `keeprun-agent uninstall`  Stop and remove the Windows service.
//! `keeprun-agent config`     Print the effective settings.
//! `keeprun-agent set-port`   Change the API port and restart the service.
//! `keeprun-agent service`    Entry point used by the Service Control Manager.

mod api;
mod copies;
mod db;
mod dpapi;
mod health;
mod job;
mod port;
mod service;
mod supervisor;
mod syspath;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use keeprun_core::{Env, Settings};
use std::sync::Arc;
use tokio::net::TcpListener;

#[derive(Parser)]
#[command(name = "keeprun-agent", version, about = "Keeprun agent")]
struct Cli {
    /// Environment: dev or prod. Defaults to KEEPRUN_ENV, then to dev for debug builds and
    /// prod for release builds.
    #[arg(long, global = true)]
    env: Option<Env>,
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run in the foreground. Ctrl+C stops the agent and every app it started.
    Run,
    /// Install the Windows service. Run from an Administrator terminal.
    Install {
        /// Do not start the service after installing it.
        #[arg(long)]
        no_start: bool,
        /// User (DOMAIN\name) who may use the desktop app. Defaults to the current user.
        #[arg(long)]
        grant_user: Option<String>,
        /// API port. Saved to the config file. Empty keeps the current port.
        #[arg(long)]
        port: Option<String>,
    },
    /// Change the API port, save it to the config file and restart the service.
    /// Run from an Administrator terminal.
    SetPort {
        port: u16,
        /// Also write the outcome to this file. Used by the desktop app, which cannot read
        /// the output of an elevated process.
        #[arg(long, hide = true)]
        report: Option<std::path::PathBuf>,
    },
    /// Stop and remove the Windows service.
    Uninstall,
    /// Print the effective settings and where they came from.
    Config,
    /// Used by the Windows Service Control Manager. Do not run by hand.
    #[command(hide = true)]
    Service,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    if let Some(env) = cli.env {
        std::env::set_var("KEEPRUN_ENV", env.as_str());
    }
    let settings = Settings::load()?;

    match cli.cmd.unwrap_or(Cmd::Run) {
        Cmd::Run => {
            init_tracing(&settings, false)?;
            let rt = tokio::runtime::Runtime::new()?;
            rt.block_on(serve(Arc::new(settings), async {
                let _ = tokio::signal::ctrl_c().await;
            }))
        }
        Cmd::Service => service::run_dispatcher(settings),
        Cmd::Install {
            no_start,
            grant_user,
            port,
        } => {
            let port = match port.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
                Some(p) => Some(p.parse::<u16>().with_context(|| {
                    format!("Port \"{p}\" is not valid. Use a number from 1 to 65535.")
                })?),
                None => None,
            };
            let grant_user = grant_user.filter(|u| !u.trim().is_empty());
            service::install(&settings, !no_start, grant_user.as_deref(), port)
        }
        Cmd::SetPort { port, report } => {
            let result = port::set_port(&settings, port);
            if let Some(path) = report {
                let text = match &result {
                    Ok(()) => "ok".to_string(),
                    Err(e) => format!("{e:#}"),
                };
                let _ = std::fs::write(path, text);
            }
            result
        }
        Cmd::Uninstall => service::uninstall(&settings),
        Cmd::Config => {
            println!("{}", serde_json::to_string_pretty(&settings)?);
            Ok(())
        }
    }
}

pub fn init_tracing(settings: &Settings, to_file: bool) -> Result<()> {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_new(&settings.log_level)
        .with_context(|| format!("Invalid log_level \"{}\"", settings.log_level))?;
    if to_file {
        std::fs::create_dir_all(&settings.data_dir)?;
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(settings.data_dir.join("agent.log"))?;
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .with_ansi(false)
            .with_writer(std::sync::Mutex::new(file))
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
    Ok(())
}

/// Start the API and supervisor, run until `shutdown` resolves, then stop every app.
pub async fn serve(
    settings: Arc<Settings>,
    shutdown: impl std::future::Future<Output = ()> + Send + 'static,
) -> Result<()> {
    std::fs::create_dir_all(&settings.data_dir)
        .with_context(|| format!("Cannot create data folder {}", settings.data_dir.display()))?;
    let token_path = settings.token_path();
    let new_token = !token_path.is_file();
    let token = keeprun_core::token::load_or_create(&token_path)?;
    if new_token && settings.env == Env::Prod {
        if let Err(e) = service::secure_token_file(&token_path, None) {
            let _ = std::fs::remove_file(&token_path);
            return Err(e);
        }
    }

    let sup = supervisor::Supervisor::load(settings.clone())?;
    let state = api::AppState {
        sup: sup.clone(),
        settings: settings.clone(),
        token: token.into(),
    };
    let addr = format!("{}:{}", settings.host, settings.port);
    let listener = TcpListener::bind(&addr).await.with_context(|| {
        format!(
            "Cannot listen on {addr}. Another agent may be running. Stop it or set KEEPRUN_PORT."
        )
    })?;

    tracing::info!(env = %settings.env, %addr, data_dir = %settings.data_dir.display(), "Keeprun agent listening");
    for src in &settings.sources {
        tracing::debug!(file = %src.display(), "config loaded");
    }
    // Only after the port is bound, so a second agent instance never starts apps.
    sup.autostart()?;
    sup.watch_copies();
    sup.watch_health();

    axum::serve(listener, api::router(state))
        .with_graceful_shutdown(shutdown)
        .await?;
    tracing::info!("shutting down, stopping apps");
    sup.stop_all().await;
    Ok(())
}
