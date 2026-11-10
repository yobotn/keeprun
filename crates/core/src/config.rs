//! Environment aware settings.
//!
//! Resolution order (later wins):
//! 1. Built-in defaults for the active environment.
//! 2. Config file: `config/<env>.toml` then `config/<env>.local.toml` in the repo for dev,
//!    `<data_dir>/config.toml` for prod, or the file named by `KEEPRUN_CONFIG`.
//! 3. Environment variables: `KEEPRUN_HOST`, `KEEPRUN_PORT`, `KEEPRUN_DATA_DIR`,
//!    `KEEPRUN_LOG`, `KEEPRUN_STOP_TIMEOUT_MS`.
//!
//! The environment is chosen by `KEEPRUN_ENV` (`dev` or `prod`). Without it, debug builds
//! use `dev` and release builds use `prod`.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Env {
    Dev,
    Prod,
}

impl Env {
    pub fn detect() -> Result<Self> {
        match var("KEEPRUN_ENV") {
            Some(v) => v.parse(),
            None => Ok(if cfg!(debug_assertions) {
                Env::Dev
            } else {
                Env::Prod
            }),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Env::Dev => "dev",
            Env::Prod => "prod",
        }
    }
}

impl fmt::Display for Env {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl std::str::FromStr for Env {
    type Err = anyhow::Error;
    fn from_str(s: &str) -> Result<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "dev" | "development" => Ok(Env::Dev),
            "prod" | "production" => Ok(Env::Prod),
            other => bail!("KEEPRUN_ENV is \"{other}\". Use \"dev\" or \"prod\"."),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Settings {
    pub env: Env,
    pub host: String,
    pub port: u16,
    pub data_dir: PathBuf,
    pub log_level: String,
    pub stop_timeout_ms: u64,
    /// Config files that were found and applied, in order.
    pub sources: Vec<PathBuf>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct FileSettings {
    host: Option<String>,
    port: Option<u16>,
    data_dir: Option<PathBuf>,
    log_level: Option<String>,
    stop_timeout_ms: Option<u64>,
}

/// Repo root, used to locate `config/` and `.dev-data/` in dev.
pub fn repo_root() -> PathBuf {
    let p = Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..");
    p.canonicalize().map(strip_unc).unwrap_or(p)
}

/// `canonicalize` on Windows returns `\\?\C:\...`. Most tools, cmd.exe included, dislike it.
fn strip_unc(p: PathBuf) -> PathBuf {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\") {
        Some(rest) if !rest.starts_with("UNC") => PathBuf::from(rest),
        _ => p,
    }
}

fn program_data() -> PathBuf {
    std::env::var_os("ProgramData")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
}

impl Settings {
    pub fn defaults(env: Env) -> Self {
        match env {
            Env::Dev => Settings {
                env,
                host: "127.0.0.1".into(),
                port: 7181,
                data_dir: repo_root().join(".dev-data"),
                log_level: "keeprun=debug,info".into(),
                stop_timeout_ms: 3000,
                sources: vec![],
            },
            Env::Prod => Settings {
                env,
                host: "127.0.0.1".into(),
                port: 7171,
                data_dir: program_data().join("Keeprun"),
                log_level: "keeprun=info,warn".into(),
                stop_timeout_ms: 10_000,
                sources: vec![],
            },
        }
    }

    /// Load settings for the detected environment.
    pub fn load() -> Result<Self> {
        Self::load_for(Env::detect()?)
    }

    pub fn load_for(env: Env) -> Result<Self> {
        let mut s = Self::defaults(env);
        if let Some(dir) = var("KEEPRUN_DATA_DIR") {
            s.data_dir = PathBuf::from(dir);
        }

        let files: Vec<PathBuf> = match var("KEEPRUN_CONFIG") {
            Some(p) => vec![PathBuf::from(p)],
            None => match env {
                Env::Dev => {
                    let dir = repo_root().join("config");
                    vec![dir.join("dev.toml"), dir.join("dev.local.toml")]
                }
                Env::Prod => vec![s.data_dir.join("config.toml")],
            },
        };
        for f in files {
            if f.is_file() {
                s.apply_file(&f)?;
                s.sources.push(f);
            }
        }

        if let Some(v) = var("KEEPRUN_HOST") {
            s.host = v;
        }
        if let Some(v) = var("KEEPRUN_PORT") {
            s.port = v.parse().with_context(|| {
                format!("KEEPRUN_PORT is \"{v}\". Use a number from 1 to 65535.")
            })?;
        }
        if let Some(v) = var("KEEPRUN_DATA_DIR") {
            s.data_dir = PathBuf::from(v);
        }
        if let Some(v) = var("KEEPRUN_LOG") {
            s.log_level = v;
        }
        if let Some(v) = var("KEEPRUN_STOP_TIMEOUT_MS") {
            s.stop_timeout_ms = v.parse().with_context(|| {
                format!("KEEPRUN_STOP_TIMEOUT_MS is \"{v}\". Use a number of milliseconds.")
            })?;
        }
        Ok(s)
    }

    fn apply_file(&mut self, path: &Path) -> Result<()> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("Cannot read {}", path.display()))?;
        let f: FileSettings = toml::from_str(&text)
            .with_context(|| format!("Invalid config file {}", path.display()))?;
        if let Some(v) = f.host {
            self.host = v;
        }
        if let Some(v) = f.port {
            self.port = v;
        }
        if let Some(v) = f.data_dir {
            self.data_dir = v;
        }
        if let Some(v) = f.log_level {
            self.log_level = v;
        }
        if let Some(v) = f.stop_timeout_ms {
            self.stop_timeout_ms = v;
        }
        Ok(())
    }

    pub fn base_url(&self) -> String {
        format!("http://{}:{}", self.host, self.port)
    }

    pub fn token_path(&self) -> PathBuf {
        self.data_dir.join("agent.token")
    }

    pub fn apps_path(&self) -> PathBuf {
        self.data_dir.join("apps.json")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.data_dir.join("logs")
    }

    /// Windows service name. Dev and prod use different names so both can be installed.
    pub fn service_name(&self) -> &'static str {
        match self.env {
            Env::Dev => "KeeprunAgentDev",
            Env::Prod => "KeeprunAgent",
        }
    }
}

fn var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_env_names() {
        assert_eq!("dev".parse::<Env>().unwrap(), Env::Dev);
        assert_eq!("Production".parse::<Env>().unwrap(), Env::Prod);
        assert!("staging".parse::<Env>().is_err());
    }

    #[test]
    fn envs_use_different_ports_and_dirs() {
        let d = Settings::defaults(Env::Dev);
        let p = Settings::defaults(Env::Prod);
        assert_ne!(d.port, p.port);
        assert_ne!(d.data_dir, p.data_dir);
        assert_ne!(d.service_name(), p.service_name());
    }

    #[test]
    fn repo_config_files_parse() {
        for name in ["dev.toml", "prod.toml"] {
            let mut s = Settings::defaults(Env::Dev);
            s.apply_file(&repo_root().join("config").join(name))
                .unwrap();
        }
    }
}
