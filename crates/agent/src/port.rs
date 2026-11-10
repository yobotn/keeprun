//! Change the port the agent listens on. The desktop app and CLI read the same config file,
//! so they follow without extra steps.

use anyhow::{bail, Context, Result};
use keeprun_core::{Env, Settings};

/// Check that `port` is free, unless the agent itself already holds it.
pub fn ensure_free(settings: &Settings, port: u16) -> Result<()> {
    if port == 0 {
        bail!("Port 0 is not valid. Use a number from 1 to 65535.");
    }
    if std::net::TcpListener::bind((settings.host.as_str(), port)).is_ok() {
        return Ok(());
    }
    let owner = crate::job::listening_ports()
        .into_iter()
        .find(|(_, p)| *p == port)
        .map(|(pid, _)| {
            let name = crate::job::process_name(pid).unwrap_or_else(|| "another process".into());
            format!(" by {name} (PID {pid})")
        })
        .unwrap_or_default();
    bail!("Port {port} is in use{owner}. Choose another port.")
}

/// Write `port` to the config file. Returns the file path.
pub fn write(settings: &Settings, port: u16) -> Result<std::path::PathBuf> {
    let path = settings.writable_config_path();
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(_) if settings.env == Env::Prod => include_str!("../../../config/prod.toml").into(),
        Err(_) => String::new(),
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(&path, keeprun_core::config::with_port(&text, port)).with_context(|| {
        format!(
            "Cannot write {}. Run this command from an Administrator terminal.",
            path.display()
        )
    })?;
    Ok(path)
}

/// `keeprun-agent set-port`: validate, save, and restart the service so it takes effect.
pub fn set_port(settings: &Settings, port: u16) -> Result<()> {
    if port == settings.port {
        println!("The agent already uses port {port}.");
        return Ok(());
    }
    ensure_free(settings, port)?;
    let path = write(settings, port)?;
    println!("Saved port {port} to {}.", path.display());
    match crate::service::restart_if_running(settings)? {
        true => println!(
            "Restarted the agent. It now listens on http://{}:{port}.",
            settings.host
        ),
        false if settings.env == Env::Dev => {
            println!("Restart the dev agent (npm run dev) to use the new port.")
        }
        false => println!("The service is not running. It uses port {port} when it starts."),
    }
    println!("The desktop app and CLI pick up the new port on their next connection.");
    Ok(())
}
