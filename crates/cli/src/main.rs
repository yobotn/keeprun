//! `keeprun` command line client. Talks to the local agent API.

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use futures_util::StreamExt;
use keeprun_core::detect;
use keeprun_core::model::{
    looks_secret, parse_dotenv, ApiError, AppState, AppView, Event, Health, IssueReport, LogLine,
    NewApp, RestartPolicy, RunTask, Stream, TaskSpec, UpdateApp,
};
use keeprun_core::{Env, Settings};
use reqwest::{Method, StatusCode};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use tokio_tungstenite::tungstenite::Message;

#[derive(Parser)]
#[command(
    name = "keeprun",
    version,
    about = "Run and manage internal apps on Windows"
)]
struct Cli {
    /// Environment: dev or prod. Defaults to KEEPRUN_ENV, then to dev for debug builds and
    /// prod for release builds.
    #[arg(long, global = true)]
    env: Option<Env>,
    /// Print JSON instead of tables.
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Show the app in this folder: state, URL, errors, and whether its database needs
    /// migrating. Outside an app folder: the agent and the active environment.
    Status {
        /// App id. Defaults to the app whose folder holds the current folder.
        app: Vec<String>,
    },
    /// List apps with status, CPU, memory and ports.
    #[command(alias = "ls")]
    List,
    /// Show what Keeprun detects in a folder.
    Detect {
        /// Folder. Defaults to the current folder.
        dir: Option<PathBuf>,
    },
    /// Add an app. Without a command, the best detected command is used.
    ///
    /// Examples:
    ///   keeprun add --cwd D:\apps\hr-portal --start
    ///   keeprun add web --cwd D:\apps\web -e PORT=3001 -- npm run start
    Add {
        /// App name. Defaults to the detected name.
        name: Option<String>,
        /// App folder. Defaults to the current folder.
        #[arg(long)]
        cwd: Option<PathBuf>,
        /// Environment variable as KEY=VALUE. Repeat for more.
        #[arg(short = 'e', long = "set")]
        vars: Vec<String>,
        /// Env file the agent reads on every start, e.g. .env.local. A bare name is looked up
        /// in the app folder. By default the folder's .env is linked, or else its first other
        /// .env.* file that is not a template such as .env.example.
        #[arg(long, conflicts_with = "no_env_file")]
        env_file: Option<PathBuf>,
        /// Do not link an env file.
        #[arg(long)]
        no_env_file: bool,
        /// Tag for grouping. Repeat for more.
        #[arg(short, long = "tag")]
        tags: Vec<String>,
        /// Restart policy: never, on-failure, always.
        #[arg(long, default_value = "on-failure")]
        restart: RestartPolicy,
        /// Do not start this app when the agent starts.
        #[arg(long)]
        no_autostart: bool,
        /// Start it right after adding.
        #[arg(long)]
        start: bool,
        /// Port the app listens on. Sets PORT. Defaults to the detected port.
        #[arg(long)]
        port: Option<u16>,
        /// Command line that starts the app.
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Change app settings.
    Set {
        /// App id. Defaults to the app whose folder holds the current folder.
        app: Vec<String>,
        #[arg(long)]
        name: Option<String>,
        #[arg(long)]
        cwd: Option<PathBuf>,
        #[arg(long)]
        command: Option<String>,
        /// never, on-failure or always.
        #[arg(long)]
        restart: Option<RestartPolicy>,
        /// Restarts in a row before giving up. 0 means no limit.
        #[arg(long)]
        max_restarts: Option<u32>,
        /// Start with the agent: true or false.
        #[arg(long)]
        autostart: Option<bool>,
        /// Replace tags (comma separated).
        #[arg(long)]
        tags: Option<String>,
        /// Port the app listens on. Sets PORT. 0 removes it.
        #[arg(long)]
        port: Option<u16>,
        /// Watch the output for errors: true or false.
        #[arg(long)]
        watch_errors: Option<bool>,
        /// Env file read on every start, e.g. .env.local. "none" unlinks it.
        #[arg(long)]
        env_file: Option<String>,
        /// HTTP health check while the app runs: a path on its port such as /health, or a
        /// local http:// URL. "none" removes it.
        #[arg(long, value_name = "PATH")]
        health: Option<String>,
        /// Restart the app when the health check fails 3 times in a row: true or false.
        #[arg(long)]
        health_restart: Option<bool>,
        /// Saved task to run before every start, e.g. migrate. The app starts only when it
        /// succeeds. Not run on restarts after a crash. "none" turns it off.
        #[arg(long, value_name = "TASK")]
        before_start: Option<String>,
    },
    /// Show, set or remove environment variables.
    Env {
        /// App id. Defaults to the app whose folder holds the current folder.
        app: Vec<String>,
        /// KEY=VALUE to set. Repeat for more.
        #[arg(short = 'e', long = "set")]
        set: Vec<String>,
        /// Mark the keys set in this call as secret.
        #[arg(long)]
        secret: bool,
        /// KEY to remove. Repeat for more.
        #[arg(long)]
        unset: Vec<String>,
        /// Import a .env file.
        #[arg(long)]
        import: Option<PathBuf>,
        /// Print secret values instead of masking them.
        #[arg(long)]
        reveal: bool,
    },
    /// Remove a stopped app.
    #[command(alias = "rm")]
    Remove { app: String },
    /// Start an app and wait until it runs. Prints its URL, or the errors of the run.
    ///
    /// Examples:
    ///   kr start
    ///   kr start hr-portal
    Start {
        /// App id. Defaults to the app whose folder holds the current folder.
        app: Vec<String>,
        /// Return without waiting for the app to run.
        #[arg(long)]
        no_wait: bool,
    },
    /// Stop an app and its process tree.
    Stop {
        /// App id. Defaults to the app whose folder holds the current folder.
        app: Vec<String>,
    },
    /// Restart an app and wait until it runs. Prints its URL, or the errors of the new run.
    ///
    /// Examples:
    ///   kr restart
    ///   kr restart the app
    ///   kr restart hr-portal
    Restart {
        /// App id. Defaults to the app whose folder holds the current folder.
        app: Vec<String>,
        /// Return without waiting for the app to run.
        #[arg(long)]
        no_wait: bool,
    },
    /// Apply database migrations: run the app's migrate task, then restart the app.
    ///
    /// The migrate task is the saved task named migrate, or else the first one whose name or
    /// command applies migrations (prisma migrate deploy, manage.py migrate, alembic upgrade,
    /// dotnet ef database update). Tasks that create, reset or roll back migrations are never
    /// picked.
    ///
    /// Examples:
    ///   kr migrate
    ///   kr migrate hr-portal --no-restart
    ///   kr migrate --dismiss
    Migrate {
        /// App id. Defaults to the app whose folder holds the current folder.
        app: Vec<String>,
        /// Migrate, but leave the app running as it is.
        #[arg(long)]
        no_restart: bool,
        /// Run nothing: the database is up to date, e.g. migrated from a terminal. Clears the
        /// "migration needed" notice.
        #[arg(long, conflicts_with = "no_restart")]
        dismiss: bool,
    },
    /// Show app logs.
    Logs {
        /// App id. Defaults to the app whose folder holds the current folder.
        app: Vec<String>,
        /// Keep streaming new lines.
        #[arg(short, long)]
        follow: bool,
        /// Number of recent lines to show.
        #[arg(short = 'n', long, default_value_t = 200)]
        tail: usize,
    },
    /// Show errors an app printed while running, as a prompt for a coding agent.
    ///
    /// Examples:
    ///   keeprun issue hr-portal
    ///   keeprun issue hr-portal --clear
    Issue {
        /// App id. Defaults to the app whose folder holds the current folder.
        app: Vec<String>,
        /// Forget the current issue. A new error starts a new one.
        #[arg(long)]
        clear: bool,
    },
    /// Show recent events for one app, or all apps.
    Events {
        app: Option<String>,
        #[arg(short = 'n', long, default_value_t = 30)]
        limit: u32,
    },
    /// List, add or remove an app's saved tasks (one-off commands such as migrations).
    ///
    /// Examples:
    ///   keeprun tasks hr-portal
    ///   keeprun tasks hr-portal --add migrate -- npm run db:migrate
    ///   keeprun tasks hr-portal --rm migrate
    Tasks {
        /// App id. Defaults to the app whose folder holds the current folder.
        app: Vec<String>,
        /// Save a task with this name. The command follows --.
        #[arg(long, value_name = "NAME", conflicts_with = "rm")]
        add: Option<String>,
        /// Remove the saved task with this name.
        #[arg(long, value_name = "NAME")]
        rm: Option<String>,
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Run a saved task, or a one-off command, in the app folder with the app environment.
    /// Shows the output and exits with the task's exit code. Ctrl+C stops the task.
    ///
    /// Examples:
    ///   keeprun run hr-portal migrate
    ///   keeprun run hr-portal -- npm run seed
    ///   kr run seed               (the app in the current folder)
    Run {
        /// App id, then the saved task name. The app defaults to the one whose folder holds
        /// the current folder. See `keeprun tasks <app>`.
        #[arg(value_name = "APP TASK")]
        words: Vec<String>,
        /// Start the task and return without waiting for it.
        #[arg(long)]
        no_wait: bool,
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Update an app from Git: fast-forward its branch, run its install task when a lockfile or
    /// other dependency list changed, then restart it if it runs. Never merges.
    ///
    /// Examples:
    ///   keeprun pull hr-portal
    ///   keeprun pull hr-portal --no-restart
    Pull {
        /// App id. Defaults to the app whose folder holds the current folder.
        app: Vec<String>,
        /// Pull, install and migrate, but leave the app running as it is.
        #[arg(long)]
        no_restart: bool,
        /// Do not run the migrate task when migrations changed.
        #[arg(long)]
        no_migrate: bool,
    },
    /// Tell coding agents in an app folder to use Keeprun: adds a short note to its CLAUDE.md
    /// or AGENTS.md (restart with `kr restart`, migrate with `kr migrate`, never start the app
    /// themselves), and adds the Keeprun MCP server to Claude Code when it is installed.
    ///
    /// Examples:
    ///   kr agent-setup
    ///   kr agent-setup hr-portal --no-mcp
    #[command(alias = "setup-agent")]
    AgentSetup {
        /// App id. Defaults to the app whose folder holds the current folder.
        app: Vec<String>,
        /// Only write the note. Do not add the MCP server to Claude Code.
        #[arg(long)]
        no_mcp: bool,
    },
    /// Run an MCP server over stdio, so coding agents check status, read logs and restart
    /// apps with typed tools. Tools default to the app in the current folder.
    ///
    /// Add it to Claude Code:
    ///   claude mcp add --scope user keeprun -- keeprun mcp
    Mcp {
        /// Only read status, logs, errors and events. No start, stop, restart or migrate.
        #[arg(long)]
        read_only: bool,
        /// Also allow running saved tasks (build, migrate, test).
        #[arg(long)]
        allow_tasks: bool,
    },
}

mod mcp;

/// Unix time in milliseconds.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

struct Client {
    http: reqwest::Client,
    settings: Settings,
    token: String,
}

impl Client {
    fn new(settings: Settings) -> Result<Self> {
        let token = keeprun_core::token::read(&settings.token_path())?;
        Ok(Client {
            http: reqwest::Client::new(),
            settings,
            token,
        })
    }

    async fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&impl serde::Serialize>,
    ) -> Result<Option<T>> {
        let url = format!("{}{}", self.settings.base_url(), path);
        let mut req = self.http.request(method, &url).bearer_auth(&self.token);
        if let Some(b) = body {
            req = req.json(b);
        }
        let res = req.send().await.map_err(|_| self.unreachable())?;
        let status = res.status();
        if status == StatusCode::NO_CONTENT {
            return Ok(None);
        }
        if !status.is_success() {
            let msg = res
                .json::<ApiError>()
                .await
                .map(|e| e.error)
                .unwrap_or_else(|_| status.to_string());
            bail!("{msg}");
        }
        Ok(Some(
            res.json()
                .await
                .context("Unexpected response from the agent")?,
        ))
    }

    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T> {
        self.call(Method::GET, path, None::<&()>)
            .await?
            .context("Empty response")
    }

    async fn send<T: serde::de::DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: &impl serde::Serialize,
    ) -> Result<T> {
        self.call(method, path, Some(body))
            .await?
            .context("Empty response")
    }

    /// Wait for a start to settle: running with a port, or not active. Up to 20 seconds, and
    /// as long as a task that runs before start keeps going, up to 10 minutes.
    pub(crate) async fn settle(&self, id: &str) -> Result<AppView> {
        let started = std::time::Instant::now();
        let mut waited = std::time::Duration::ZERO;
        let mut last = None;
        while waited < std::time::Duration::from_secs(20)
            && started.elapsed() < std::time::Duration::from_secs(600)
        {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            let a: AppView = self.get(&format!("/apps/{}", enc(id))).await?;
            let listening = a.stats.as_ref().is_some_and(|s| !s.ports.is_empty());
            let prestart = a.state == AppState::Starting
                && a.pid.is_none()
                && a.task.as_ref().is_some_and(|t| t.running);
            if !prestart {
                waited += std::time::Duration::from_millis(500);
            }
            let done = match a.state {
                AppState::Running => {
                    listening || a.issue.is_some() || waited >= std::time::Duration::from_secs(10)
                }
                AppState::Starting | AppState::Stopping => false,
                _ => true,
            };
            if done {
                return Ok(a);
            }
            last = Some(a);
        }
        last.context("The app did not answer.")
    }

    fn unreachable(&self) -> anyhow::Error {
        anyhow::anyhow!(
            "Cannot reach the {} agent at {}. Start it with `keeprun-agent run` or start the service.",
            self.settings.env,
            self.settings.base_url()
        )
    }
}

/// Words that may surround an app id, so `kr restart the app` works as `kr restart`.
const FILLER: &[&str] = &[
    "the", "this", "my", "app", "it", "please", "now", "again", "here", "server", "service",
];

/// The app whose folder is the current folder or holds it. The deepest wins.
pub(crate) fn owner<'a>(apps: &'a [AppView], cwd: &std::path::Path) -> Option<&'a AppView> {
    let key = |p: &std::path::Path| {
        let mut s = p.to_string_lossy().replace('/', "\\").to_lowercase();
        if !s.ends_with('\\') {
            s.push('\\');
        }
        s
    };
    let here = key(cwd);
    apps.iter()
        .filter(|a| here.starts_with(&key(&a.spec.cwd)))
        .max_by_key(|a| a.spec.cwd.as_os_str().len())
}

/// The app named by the first word, or else the app in the current folder, and the words
/// left over. Filler words such as "the app" are skipped.
async fn target(c: &Client, words: &[String]) -> Result<(AppView, Vec<String>)> {
    let apps: Vec<AppView> = c.get("/apps").await?;
    let is_app = |w: &str| {
        apps.iter()
            .find(|a| a.spec.id.eq_ignore_ascii_case(w) || a.spec.name.eq_ignore_ascii_case(w))
    };
    let words: Vec<&String> = words
        .iter()
        .filter(|w| is_app(w).is_some() || !FILLER.contains(&w.to_lowercase().as_str()))
        .collect();
    if let Some(a) = words.first().and_then(|w| is_app(w)) {
        return Ok((
            a.clone(),
            words[1..].iter().map(|w| w.to_string()).collect(),
        ));
    }
    let cwd = std::env::current_dir().unwrap_or_default();
    let ids = || {
        if apps.is_empty() {
            "none yet, add one with: keeprun add --cwd <folder>".to_string()
        } else {
            apps.iter()
                .map(|a| a.spec.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        }
    };
    match owner(&apps, &cwd) {
        Some(a) => Ok((a.clone(), words.iter().map(|w| w.to_string()).collect())),
        None if words.is_empty() => bail!(
            "No app runs from {}. Name one, e.g. kr restart <app>. Apps: {}.",
            cwd.display(),
            ids()
        ),
        None => bail!("No app with id \"{}\". Apps: {}.", words[0], ids()),
    }
}

/// The one app the words name. Words left over are an error.
async fn one(c: &Client, words: &[String]) -> Result<AppView> {
    let (a, rest) = target(c, words).await?;
    if let Some(w) = rest.first() {
        bail!(
            "No app with id \"{w}\". Run `keeprun list` to see app ids, or leave it out to use {}.",
            a.spec.id
        );
    }
    Ok(a)
}

fn url(a: &AppView) -> Option<String> {
    a.stats
        .as_ref()
        .and_then(|s| s.ports.first())
        .map(|p| format!("http://localhost:{p}"))
}

/// What the migration notice says, in one line.
fn migration_text(a: &AppView) -> Option<String> {
    let m = a.migration.as_ref()?;
    let why = match (&m.error, m.new_files.len()) {
        (Some(e), _) => format!("the app printed: {e}"),
        (None, 1) => format!("new migration {}", m.new_files[0]),
        (None, n) => format!("{n} new migrations, e.g. {}", m.new_files[0]),
    };
    let fix = match &m.task {
        Some(_) => format!("Run: kr migrate {}", a.spec.id),
        None => format!(
            "Add a migrate task first: kr tasks {} --add migrate -- <command>",
            a.spec.id
        ),
    };
    Some(format!("Database needs migrating: {why}. {fix}"))
}

/// Print where an app stands: its URL, or why it does not run, and what needs doing. With
/// `since`, the output of a run that did not stay up, from that time on. Returns false when
/// the app does not run.
async fn report(c: &Client, a: &AppView, since: Option<u64>) -> Result<bool> {
    let id = &a.spec.id;
    let pid = a.pid.map(|p| format!(" (PID {p})")).unwrap_or_default();
    let running = a.state == AppState::Running;
    match url(a) {
        Some(u) if running => println!("{id}: Running on {u}{pid}"),
        _ => println!("{id}: {}{pid}", status_text(a)),
    }
    if let Some(i) = &a.issue {
        println!("Errors in this run: {}. Details: kr issue {id}", i.summary);
    }
    if let Some(m) = migration_text(a) {
        println!("{m}");
    }
    if !running && a.state != AppState::Starting {
        let Some(since) = since else {
            return Ok(false);
        };
        let lines: Vec<LogLine> = c
            .get(&format!("/apps/{}/logs/recent?tail=30", enc(id)))
            .await?;
        let new: Vec<&LogLine> = lines.iter().filter(|l| l.ts >= since && !l.task).collect();
        if !new.is_empty() {
            println!("Output of the run:");
            new.iter().for_each(|l| print_line(l));
        }
        return Ok(false);
    }
    Ok(true)
}

/// Percent-encode an app id for a URL path.
pub(crate) fn enc(id: &str) -> String {
    id.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn print_line(l: &LogLine) {
    match l.stream {
        Stream::Stderr => eprintln!("{}", l.text),
        Stream::System => println!("-- {}", l.text),
        Stream::Stdout => println!("{}", l.text),
    }
}

fn mb(bytes: u64) -> String {
    format!("{:.0} MB", bytes as f64 / 1024.0 / 1024.0)
}

fn status_text(a: &AppView) -> String {
    if a.spec.disabled {
        return "Disabled".into();
    }
    match (a.state, a.exit_code) {
        (AppState::Exited, Some(c)) => format!("Exited {c}"),
        (AppState::Restarting, _) => format!("Restarting ({})", a.restarts),
        (AppState::Running, _) if a.health.as_ref().is_some_and(|h| !h.ok) => {
            "Running, unhealthy".into()
        }
        (s, _) if a.issue.is_some() => format!("{s}, errors"),
        (s, _) if a.migration.as_ref().is_some_and(|m| m.error.is_some()) => {
            format!("{s}, migrate")
        }
        (s, _) => s.to_string(),
    }
}

fn print_apps(apps: &[AppView]) {
    if apps.is_empty() {
        println!("No apps yet. Add one with: keeprun add --cwd <folder>");
        return;
    }
    let w = apps
        .iter()
        .map(|a| a.spec.id.len())
        .max()
        .unwrap_or(2)
        .max(2);
    println!(
        "{:<w$}  {:<16}  {:>6}  {:>5}  {:>8}  {:<12}  COMMAND",
        "ID", "STATUS", "PID", "CPU", "MEMORY", "PORTS"
    );
    for a in apps {
        let pid = a.pid.map(|p| p.to_string()).unwrap_or_else(|| "-".into());
        let (cpu, mem, ports) = match &a.stats {
            Some(s) => (
                format!("{:.0}%", s.cpu_percent),
                mb(s.memory_bytes),
                s.ports
                    .iter()
                    .map(|p| p.to_string())
                    .collect::<Vec<_>>()
                    .join(","),
            ),
            None => ("-".into(), "-".into(), String::new()),
        };
        let ports = if ports.is_empty() { "-".into() } else { ports };
        println!(
            "{:<w$}  {:<16}  {:>6}  {:>5}  {:>8}  {:<12}  {}",
            a.spec.id,
            status_text(a),
            pid,
            cpu,
            mem,
            ports,
            a.spec.command
        );
    }
}

fn time(ts: u64) -> String {
    keeprun_core::model::format_utc(ts)
}

fn parse_vars(vars: &[String]) -> Result<BTreeMap<String, String>> {
    let mut env = BTreeMap::new();
    for v in vars {
        let (k, val) = v
            .split_once('=')
            .with_context(|| format!("\"{v}\" is not KEY=VALUE."))?;
        env.insert(k.trim().to_string(), val.to_string());
    }
    Ok(env)
}

/// The name to store for an env file: relative to the app folder when it is inside it,
/// otherwise absolute. The agent resolves it against the app folder.
fn link_env_file(cwd: &std::path::Path, f: &std::path::Path) -> Result<String> {
    let in_app = cwd.join(f);
    let path = if f.is_relative() && in_app.is_file() {
        in_app
    } else {
        std::path::absolute(f)?
    };
    anyhow::ensure!(path.is_file(), "Env file {} does not exist.", f.display());
    Ok(match path.strip_prefix(cwd) {
        Ok(rel) => rel.display().to_string(),
        Err(_) => path.display().to_string(),
    })
}

fn read_env_file(p: &PathBuf) -> Result<BTreeMap<String, String>> {
    let text =
        std::fs::read_to_string(p).with_context(|| format!("Cannot read {}", p.display()))?;
    parse_dotenv(&text).map_err(|e| anyhow::anyhow!("{}: {e}", p.display()))
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();
    if let Some(env) = cli.env {
        std::env::set_var("KEEPRUN_ENV", env.as_str());
    }
    let settings = Settings::load()?;
    let json = cli.json;

    match &cli.cmd {
        Cmd::Status { app } => {
            // In an app folder, or with an app named: that app.
            if let Ok(c) = Client::new(settings.clone()) {
                match one(&c, app).await {
                    Ok(a) => {
                        if json {
                            println!("{}", serde_json::to_string_pretty(&a)?);
                            return Ok(());
                        }
                        report(&c, &a, None).await?;
                        if let Some(h) = a.health.as_ref().filter(|h| !h.ok) {
                            println!("Health check failing: {}", h.detail);
                        }
                        if !a.outside.is_empty() {
                            println!(
                                "A copy runs outside Keeprun (PID {}). kr start {} ends it.",
                                a.outside[0].pid, a.spec.id
                            );
                        }
                        if let Some(t) = a.task.as_ref().filter(|t| t.running) {
                            println!("Task running: {}", t.name.as_deref().unwrap_or(&t.command));
                        }
                        return Ok(());
                    }
                    Err(e) if !app.is_empty() => return Err(e),
                    Err(_) => {}
                }
            }
            let url = format!("{}/health", settings.base_url());
            println!("Environment  {}", settings.env);
            println!("Agent URL    {}", settings.base_url());
            println!("Data folder  {}", settings.data_dir.display());
            match reqwest::get(&url).await {
                Ok(r) => {
                    let h: Health = r.json().await?;
                    println!("Agent        running (v{}, env {})", h.version, h.env);
                }
                Err(_) => println!("Agent        not reachable"),
            }
            return Ok(());
        }
        Cmd::Detect { dir } => {
            let dir = dir.clone().map_or_else(std::env::current_dir, Ok)?;
            let d = detect::detect(&std::path::absolute(&dir)?).map_err(anyhow::Error::msg)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&d)?);
                return Ok(());
            }
            println!("Type     {:?}", d.kind);
            println!("Name     {}", d.name);
            if let Some(p) = d.port {
                println!("Port     {p}");
            }
            println!("Commands");
            for (i, s) in d.suggestions.iter().enumerate() {
                println!(
                    "  {}{}  {}",
                    if i == 0 { "* " } else { "  " },
                    s.label,
                    s.command
                );
            }
            if !d.tasks.is_empty() {
                println!("Tasks (saved with the app, run on demand)");
                for t in &d.tasks {
                    println!("    {:<16}  {}", t.name, t.command);
                }
            }
            let default_env = d.default_env_file().map(|f| f.name.clone());
            for f in &d.env_files {
                let mark = if Some(&f.name) == default_env.as_ref() {
                    "*"
                } else {
                    " "
                };
                println!("Env    {mark} {:<16}  {} variables", f.name, f.vars.len());
            }
            for w in d.warnings {
                println!("Warning  {w}");
            }
            return Ok(());
        }
        _ => {}
    }

    let c = Client::new(settings)?;
    match cli.cmd {
        Cmd::Status { .. } | Cmd::Detect { .. } => unreachable!(),
        Cmd::Mcp {
            read_only,
            allow_tasks,
        } => {
            let access = mcp::Access {
                control: !read_only,
                tasks: allow_tasks && !read_only,
            };
            mcp::serve(c, access).await?;
        }
        Cmd::List => {
            let apps: Vec<AppView> = c.get("/apps").await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&apps)?);
            } else {
                print_apps(&apps);
            }
        }
        Cmd::Add {
            name,
            cwd,
            vars,
            env_file,
            no_env_file,
            tags,
            restart,
            no_autostart,
            start,
            port,
            command,
        } => {
            let cwd = cwd.map_or_else(std::env::current_dir, Ok)?;
            let cwd = std::path::absolute(&cwd).unwrap_or(cwd);
            let mut env = BTreeMap::new();
            // Detected one-off commands (install, migrate, build) are saved as tasks, never run.
            let detected = detect::detect(&cwd).ok();
            let detected_port = detected.as_ref().and_then(|d| d.port);
            let env_file = match (env_file, no_env_file) {
                (_, true) => None,
                (Some(f), _) => Some(link_env_file(&cwd, &f)?),
                (None, _) => detected
                    .as_ref()
                    .and_then(|d| d.default_env_file())
                    .map(|f| f.name.clone()),
            };
            let file_vars = match &env_file {
                Some(f) => {
                    let vars =
                        keeprun_core::model::read_env_file(&cwd, f).map_err(anyhow::Error::msg)?;
                    println!(
                        "Env file: {f} ({} variables, read on every start). Pick another with --env-file or none with --no-env-file.",
                        vars.len()
                    );
                    vars
                }
                None => BTreeMap::new(),
            };
            let tasks: Vec<TaskSpec> = detected
                .map(|d| d.tasks)
                .unwrap_or_default()
                .into_iter()
                .map(|t| TaskSpec {
                    name: if t.name.is_empty() { t.label } else { t.name },
                    command: t.command,
                })
                .collect();
            let (name, command) = match (name, command.is_empty()) {
                (Some(name), false) => (name, command.join(" ")),
                (name, _) => {
                    let d = detect::detect(&cwd).map_err(anyhow::Error::msg)?;
                    for w in &d.warnings {
                        eprintln!("Warning: {w}");
                    }
                    let command = if command.is_empty() {
                        let best = d.suggestions.first().with_context(|| {
                            format!(
                                "Nothing to run was detected in {}. Pass the command after --.",
                                cwd.display()
                            )
                        })?;
                        println!("Detected: {}", best.label);
                        best.command.clone()
                    } else {
                        command.join(" ")
                    };
                    (name.unwrap_or(d.name), command)
                }
            };
            env.extend(parse_vars(&vars)?);
            // Detected commands may read %PORT%, so give the app a port whenever one is known.
            // A PORT in the env file is left to the file, so editing it there takes effect.
            if let Some(p) = port.filter(|p| *p != 0) {
                env.insert("PORT".into(), p.to_string());
            } else if let Some(p) = detected_port
                .filter(|_| !env.contains_key("PORT") && !file_vars.contains_key("PORT"))
            {
                env.insert("PORT".into(), p.to_string());
            }
            let secrets: BTreeSet<String> =
                env.keys().filter(|k| looks_secret(k)).cloned().collect();
            let body = NewApp {
                name,
                cwd,
                command,
                env,
                secrets,
                tags,
                autostart: !no_autostart,
                restart,
                max_restarts: 5,
                tasks,
                watch_errors: true,
                env_file,
            };
            let app: AppView = c.send(Method::POST, "/apps", &body).await?;
            println!("Added {} ({}).", app.spec.id, app.spec.command);
            if !app.spec.tasks.is_empty() {
                let names: Vec<&str> = app.spec.tasks.iter().map(|t| t.name.as_str()).collect();
                println!(
                    "Saved tasks: {}. Run one with: keeprun run {} <task>",
                    names.join(", "),
                    app.spec.id
                );
            }
            if start {
                action(&c, &app.spec.id, "start").await?;
            } else {
                println!("Start it with: keeprun start {}", app.spec.id);
            }
        }
        Cmd::Set {
            app,
            name,
            cwd,
            command,
            restart,
            max_restarts,
            autostart,
            tags,
            port,
            watch_errors,
            env_file,
            health,
            health_restart,
            before_start,
        } => {
            let app = one(&c, &app).await?.spec.id;
            let needs_restart =
                cwd.is_some() || command.is_some() || port.is_some() || env_file.is_some();
            let env = match port {
                Some(p) => {
                    let v: AppView = c.get(&format!("/apps/{app}")).await?;
                    let mut env = v.spec.env;
                    if p == 0 {
                        env.remove("PORT");
                    } else {
                        env.insert("PORT".into(), p.to_string());
                    }
                    Some(env)
                }
                None => None,
            };
            let u = UpdateApp {
                name,
                cwd: cwd.map(|p| std::path::absolute(&p).unwrap_or(p)),
                command,
                restart,
                max_restarts,
                autostart,
                tags: tags.map(|t| t.split(',').map(str::to_string).collect()),
                env,
                watch_errors,
                env_file: env_file.map(|f| {
                    if f.eq_ignore_ascii_case("none") {
                        String::new()
                    } else {
                        f
                    }
                }),
                health_check: health.map(|h| {
                    if h.eq_ignore_ascii_case("none") {
                        String::new()
                    } else {
                        h
                    }
                }),
                health_restart,
                before_start: before_start.map(|t| {
                    if t.eq_ignore_ascii_case("none") {
                        String::new()
                    } else {
                        t
                    }
                }),
                ..Default::default()
            };
            let v: AppView = c.send(Method::PATCH, &format!("/apps/{app}"), &u).await?;
            println!("Saved {}.", v.spec.id);
            if let Some(t) = &v.spec.before_start {
                println!("Task {t} runs before every start. The app starts only when it succeeds.");
            }
            if needs_restart && v.state.is_active() {
                println!("Restart to apply: keeprun restart {}", v.spec.id);
            }
        }
        Cmd::Env {
            app,
            set,
            secret,
            unset,
            import,
            reveal,
        } => {
            let v = one(&c, &app).await?;
            let app = v.spec.id.clone();
            let mut env = v.spec.env.clone();
            let mut secrets = v.spec.secrets.clone();
            let changing = !set.is_empty() || !unset.is_empty() || import.is_some();
            if let Some(f) = &import {
                let vars = read_env_file(f)?;
                secrets.extend(vars.keys().filter(|k| looks_secret(k)).cloned());
                env.extend(vars);
            }
            let vars = parse_vars(&set)?;
            if secret {
                secrets.extend(vars.keys().cloned());
            }
            env.extend(vars);
            for k in &unset {
                env.remove(k);
                secrets.remove(k);
            }
            let v = if changing {
                let u = UpdateApp {
                    env: Some(env),
                    secrets: Some(secrets),
                    ..Default::default()
                };
                let v: AppView = c.send(Method::PATCH, &format!("/apps/{app}"), &u).await?;
                println!("Saved. Restart the app to apply.");
                v
            } else {
                v
            };
            if let (Some(f), Some(st)) = (&v.spec.env_file, &v.env_file_status) {
                match &st.error {
                    Some(e) => println!("# Env file {f}: {e}"),
                    None => println!("# Env file {f}: {}", st.keys.join(", ")),
                }
                if !st.changed.is_empty() {
                    println!(
                        "# Changed since start: {}. Apply with: keeprun restart {}",
                        st.changed.join(", "),
                        v.spec.id
                    );
                }
                if !v.spec.env.is_empty() {
                    println!("# Set in Keeprun (override the file):");
                }
            }
            for (k, val) in &v.spec.env {
                let shown = if v.spec.secrets.contains(k) && !reveal {
                    "********"
                } else {
                    val
                };
                println!("{k}={shown}");
            }
        }
        Cmd::Remove { app } => {
            c.call::<serde_json::Value>(Method::DELETE, &format!("/apps/{app}"), None::<&()>)
                .await?;
            println!("Removed {app}.");
        }
        Cmd::Start { app, no_wait } => {
            let a = one(&c, &app).await?;
            start(&c, &a.spec.id, "start", no_wait).await?
        }
        Cmd::Stop { app } => {
            let a = one(&c, &app).await?;
            action(&c, &a.spec.id, "stop").await?
        }
        Cmd::Restart { app, no_wait } => {
            let a = one(&c, &app).await?;
            start(&c, &a.spec.id, "restart", no_wait).await?
        }
        Cmd::Migrate {
            app,
            no_restart,
            dismiss,
        } => {
            let a = one(&c, &app).await?;
            if dismiss {
                c.call::<AppView>(
                    Method::DELETE,
                    &format!("/apps/{}/migration", enc(&a.spec.id)),
                    None::<&()>,
                )
                .await?;
                println!("Marked {} as migrated.", a.spec.id);
                return Ok(());
            }
            migrate(&c, &a, !no_restart).await?;
        }
        Cmd::AgentSetup { app, no_mcp } => {
            let a = one(&c, &app).await?;
            agent_setup(&a, no_mcp)?;
        }
        Cmd::Issue { app, clear } => {
            let app = one(&c, &app).await?.spec.id;
            if clear {
                c.call::<AppView>(Method::DELETE, &format!("/apps/{app}/issue"), None::<&()>)
                    .await?;
                println!("Cleared. A new error starts a new issue.");
                return Ok(());
            }
            let r: IssueReport = c.get(&format!("/apps/{app}/issue")).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&r)?);
            } else if let Some(p) = r.prompt {
                print!("{p}");
            } else {
                println!("No errors in the output of {app}.");
            }
        }
        Cmd::Events { app, limit } => {
            let path = match &app {
                Some(a) => format!("/apps/{a}/events?limit={limit}"),
                None => format!("/events?limit={limit}"),
            };
            let mut events: Vec<Event> = c.get(&path).await?;
            if json {
                println!("{}", serde_json::to_string_pretty(&events)?);
                return Ok(());
            }
            events.reverse();
            for e in events {
                if app.is_some() {
                    println!("{}  {}", time(e.ts), e.message);
                } else {
                    println!("{}  {:<16}  {}", time(e.ts), e.app_id, e.message);
                }
            }
        }
        Cmd::Tasks {
            app,
            add,
            rm,
            command,
        } => {
            let v = one(&c, &app).await?;
            let app = v.spec.id.clone();
            let mut tasks = v.spec.tasks.clone();
            let changed = if let Some(name) = add {
                if command.is_empty() {
                    bail!("Give the task command after --, e.g. keeprun tasks {app} --add {name} -- npm run migrate");
                }
                let command = command.join(" ");
                match tasks.iter_mut().find(|t| t.name == name) {
                    Some(t) => t.command = command,
                    None => tasks.push(TaskSpec { name, command }),
                }
                true
            } else if let Some(name) = rm {
                let before = tasks.len();
                tasks.retain(|t| t.name != name);
                if tasks.len() == before {
                    bail!("App \"{app}\" has no task named \"{name}\".");
                }
                true
            } else {
                false
            };
            let v = if changed {
                let u = UpdateApp {
                    tasks: Some(tasks),
                    ..Default::default()
                };
                c.send::<AppView>(Method::PATCH, &format!("/apps/{app}"), &u)
                    .await?
            } else {
                v
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&v.spec.tasks)?);
                return Ok(());
            }
            if v.spec.tasks.is_empty() {
                println!(
                    "No saved tasks. Add one with: keeprun tasks {app} --add <name> -- <command>"
                );
            } else {
                let w = v
                    .spec
                    .tasks
                    .iter()
                    .map(|t| t.name.len())
                    .max()
                    .unwrap_or(4)
                    .max(4);
                println!("{:<w$}  COMMAND", "NAME");
                for t in &v.spec.tasks {
                    println!("{:<w$}  {}", t.name, t.command);
                }
            }
            if let Some(t) = keeprun_core::migrate::task(&v.spec.tasks) {
                println!("\nMigrate task: {} (kr migrate)", t.name);
            }
            if let Some(t) = &v.spec.before_start {
                println!("Runs before every start: {t}");
            }
            if let Some(r) = &v.task {
                let what = r.name.as_deref().unwrap_or(&r.command);
                let state = match (r.running, r.exit_code) {
                    (true, _) => "running".to_string(),
                    (false, Some(code)) => format!("exit code {code}"),
                    (false, None) => "ended".to_string(),
                };
                println!(
                    "\nLast run: {what}, {state}, started {}",
                    time(r.started_at)
                );
            }
        }
        Cmd::Pull {
            app,
            no_restart,
            no_migrate,
        } => {
            let v = one(&c, &app).await?;
            let r = keeprun_core::git::pull(&v.spec.cwd, true).map_err(|e| match e.as_str() {
                "no-git" => anyhow::anyhow!("Git is not installed or not on PATH."),
                _ => anyhow::anyhow!("git pull in {}: {e}", v.spec.cwd.display()),
            })?;
            let branch = r.status.branch.as_deref().unwrap_or("HEAD");
            if r.commits == 0 {
                println!("{} is up to date ({branch}).", v.spec.id);
                return Ok(());
            }
            println!("Pulled {branch}. New commits: {}.", r.commits);
            if r.dependencies {
                if v.spec.tasks.iter().any(|t| t.name == "install") {
                    println!("Dependencies changed. Running the install task.");
                    let body = RunTask {
                        name: Some("install".into()),
                        command: None,
                        restart: false,
                    };
                    let code = run_task(&c, &v.spec.id, &body, false).await?;
                    if code != 0 {
                        bail!("The install task failed with exit code {code}. The app was not restarted.");
                    }
                } else {
                    println!(
                        "Dependencies changed. Install them: {} has no install task (keeprun tasks {} --add install -- <command>).",
                        v.spec.id, v.spec.id
                    );
                }
            }
            if r.migrations && !no_migrate {
                match keeprun_core::migrate::task(&v.spec.tasks) {
                    Some(t) => {
                        println!("Migrations changed. Running the {} task.", t.name);
                        let body = RunTask {
                            name: Some(t.name.clone()),
                            command: None,
                            restart: false,
                        };
                        let code = run_task(&c, &v.spec.id, &body, false).await?;
                        if code != 0 {
                            bail!("The {} task failed with exit code {code}. The app was not restarted.", t.name);
                        }
                    }
                    None => println!(
                        "Migrations changed. Apply them: {} has no migrate task (kr tasks {} --add migrate -- <command>).",
                        v.spec.id, v.spec.id
                    ),
                }
            }
            if !no_restart && v.state.is_active() {
                start(&c, &v.spec.id, "restart", false).await?;
            }
        }
        Cmd::Run {
            words,
            no_wait,
            command,
        } => {
            let (a, rest) = target(&c, &words).await?;
            let app = a.spec.id;
            if rest.len() > 1 {
                bail!("Give one task name, e.g. kr run {app} {}.", rest[0]);
            }
            let task = rest.into_iter().next();
            let body = match (task, command.is_empty()) {
                (Some(name), true) => RunTask {
                    name: Some(name),
                    command: None,
                    restart: false,
                },
                (None, false) => RunTask {
                    name: None,
                    command: Some(command.join(" ")),
                    restart: false,
                },
                (Some(_), false) => bail!("Give a task name or a command after --, not both."),
                (None, true) => bail!(
                    "Give a task name or a command, e.g. keeprun run {app} migrate. See: keeprun tasks {app}"
                ),
            };
            let code = run_task(&c, &app, &body, no_wait).await?;
            if code != 0 {
                std::process::exit(code);
            }
        }
        Cmd::Logs { app, follow, tail } => {
            let app = one(&c, &app).await?.spec.id;
            if !follow {
                let lines: Vec<LogLine> = c
                    .get(&format!("/apps/{app}/logs/recent?tail={tail}"))
                    .await?;
                lines.iter().for_each(print_line);
                return Ok(());
            }
            let url = format!(
                "ws://{}:{}/apps/{}/logs?token={}",
                c.settings.host, c.settings.port, app, c.token
            );
            let (mut ws, _) =
                tokio_tungstenite::connect_async(&url)
                    .await
                    .map_err(|e| match e {
                        tokio_tungstenite::tungstenite::Error::Http(r) if r.status() == 404 => {
                            anyhow::anyhow!(
                                "No app with id \"{app}\". Run `keeprun list` to see app ids."
                            )
                        }
                        _ => c.unreachable(),
                    })?;
            // The agent sends its buffer first. Show only the last `tail` lines of it.
            let mut history = Vec::new();
            loop {
                tokio::select! {
                    msg = ws.next() => match msg {
                        Some(Ok(Message::Text(t))) => {
                            if let Ok(l) = serde_json::from_str::<LogLine>(&t) { history.push(l) }
                        }
                        Some(Ok(_)) => {}
                        _ => return Ok(()),
                    },
                    _ = tokio::time::sleep(std::time::Duration::from_millis(300)) => break,
                }
            }
            history
                .iter()
                .skip(history.len().saturating_sub(tail))
                .for_each(print_line);
            loop {
                tokio::select! {
                    msg = ws.next() => match msg {
                        Some(Ok(Message::Text(t))) => {
                            if let Ok(l) = serde_json::from_str::<LogLine>(&t) { print_line(&l) }
                        }
                        Some(Ok(_)) => {}
                        _ => { println!("-- Connection to the agent closed."); return Ok(()) }
                    },
                    _ = tokio::signal::ctrl_c() => return Ok(()),
                }
            }
        }
    }
    Ok(())
}

/// Start a task and stream its output until it ends. Returns the exit code to exit with.
async fn run_task(c: &Client, app: &str, body: &RunTask, no_wait: bool) -> Result<i32> {
    if no_wait {
        let v: AppView = c
            .send(Method::POST, &format!("/apps/{app}/tasks/run"), body)
            .await?;
        if let Some(r) = v.task {
            println!(
                "Started {}. Output: keeprun logs {app} -f",
                r.name.unwrap_or(r.command)
            );
        }
        return Ok(0);
    }
    // Connect first so no output is missed, then skip the history the agent sends.
    let url = format!(
        "ws://{}:{}/apps/{}/logs?token={}",
        c.settings.host, c.settings.port, app, c.token
    );
    let (mut ws, _) = tokio_tungstenite::connect_async(&url)
        .await
        .map_err(|e| match e {
            tokio_tungstenite::tungstenite::Error::Http(r) if r.status() == 404 => {
                anyhow::anyhow!("No app with id \"{app}\". Run `keeprun list` to see app ids.")
            }
            _ => c.unreachable(),
        })?;
    loop {
        tokio::select! {
            msg = ws.next() => if msg.is_none() { return Err(c.unreachable()) },
            _ = tokio::time::sleep(std::time::Duration::from_millis(200)) => break,
        }
    }
    let v: AppView = c
        .send(Method::POST, &format!("/apps/{app}/tasks/run"), body)
        .await?;
    let started = v.task.map(|t| t.started_at).unwrap_or(0);
    let mut interrupted = false;
    loop {
        tokio::select! {
            msg = ws.next() => match msg {
                Some(Ok(Message::Text(t))) => {
                    let Ok(l) = serde_json::from_str::<LogLine>(&t) else { continue };
                    if !l.task || l.ts < started {
                        continue;
                    }
                    print_line(&l);
                    if l.stream == Stream::System
                        && ["Task finished", "Task failed", "Task stopped"]
                            .iter()
                            .any(|p| l.text.starts_with(p))
                    {
                        break;
                    }
                }
                Some(Ok(_)) => {}
                _ => bail!("Connection to the agent closed. The task may still be running."),
            },
            _ = tokio::signal::ctrl_c(), if !interrupted => {
                interrupted = true;
                let _ = c
                    .call::<AppView>(Method::POST, &format!("/apps/{app}/tasks/stop"), None::<&()>)
                    .await;
            }
        }
    }
    let v: AppView = c.get(&format!("/apps/{app}")).await?;
    Ok(match v.task {
        Some(r) if !r.running => r.exit_code.unwrap_or(1),
        _ => 1,
    })
}

/// Start or restart, then wait until the app runs and print its URL or what went wrong.
/// Exits with 1 when it does not run.
async fn start(c: &Client, app: &str, verb: &str, no_wait: bool) -> Result<()> {
    if no_wait {
        return action(c, app, verb).await;
    }
    let since = now_ms();
    let v: AppView = c
        .call(
            Method::POST,
            &format!("/apps/{}/{verb}", enc(app)),
            None::<&()>,
        )
        .await?
        .context("Empty response")?;
    if let Some(t) = v
        .spec
        .before_start
        .as_ref()
        .filter(|_| v.state == AppState::Starting)
    {
        println!("Running task {t} before start.");
    }
    let a = c.settle(app).await?;
    if !report(c, &a, Some(since)).await? {
        std::process::exit(1);
    }
    Ok(())
}

/// Run the app's migrate task with its output, then restart the app, or start it when it
/// ended or failed, for example on the missing migration.
async fn migrate(c: &Client, a: &AppView, restart: bool) -> Result<()> {
    let id = &a.spec.id;
    let Some(t) = keeprun_core::migrate::task(&a.spec.tasks) else {
        bail!(
            "{id} has no migrate task. Add one, e.g.: kr tasks {id} --add migrate -- npx prisma migrate deploy"
        );
    };
    println!("Running {}: {}", t.name, t.command);
    let body = RunTask {
        name: Some(t.name.clone()),
        command: None,
        restart: false,
    };
    let code = run_task(c, id, &body, false).await?;
    if code != 0 {
        eprintln!(
            "The {} task failed with exit code {code}. The app was not restarted.",
            t.name
        );
        std::process::exit(code);
    }
    if !restart {
        return Ok(());
    }
    let now: AppView = c.get(&format!("/apps/{}", enc(id))).await?;
    match now.state {
        s if s.is_active() => start(c, id, "restart", false).await,
        AppState::Failed | AppState::Exited => start(c, id, "start", false).await,
        _ => {
            println!("{id} is stopped. Start it with: kr start {id}");
            Ok(())
        }
    }
}

/// Write the note for coding agents, and add the MCP server to Claude Code when it is
/// installed and does not have it yet.
fn agent_setup(a: &AppView, no_mcp: bool) -> Result<()> {
    use keeprun_core::agent_note::{write, Written};
    let cwd = &a.spec.cwd;
    match write(cwd, &a.spec.id).map_err(anyhow::Error::msg)? {
        Written::Added(f) => println!("Added the Keeprun note to {}.", cwd.join(f).display()),
        Written::Updated(f) => println!("Updated the Keeprun note in {}.", cwd.join(f).display()),
        Written::Unchanged(f) => {
            println!("{} already has the Keeprun note.", cwd.join(f).display())
        }
    }
    if detect::which("kr").is_none() {
        println!(
            "Note: `kr` is not on PATH. Installed Keeprun adds it next to keeprun.exe; until then agents can use `keeprun`."
        );
    }
    if no_mcp {
        return Ok(());
    }
    let Some(claude) = detect::which("claude") else {
        println!("Claude Code is not installed, so no MCP server was added. Other agents: see `keeprun mcp --help`.");
        return Ok(());
    };
    let has = std::process::Command::new(&claude)
        .args(["mcp", "get", "keeprun"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if has {
        println!("Claude Code already has the Keeprun MCP server.");
        return Ok(());
    }
    let ok = std::process::Command::new(&claude)
        .args([
            "mcp", "add", "--scope", "user", "keeprun", "--", "keeprun", "mcp",
        ])
        .stdin(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if ok {
        println!("Added the Keeprun MCP server to Claude Code. It applies to new sessions.");
    } else {
        println!("Could not add the MCP server. Add it yourself: claude mcp add --scope user keeprun -- keeprun mcp");
    }
    Ok(())
}

async fn action(c: &Client, app: &str, verb: &str) -> Result<()> {
    let v: AppView = c
        .call(
            Method::POST,
            &format!("/apps/{}/{verb}", enc(app)),
            None::<&()>,
        )
        .await?
        .context("Empty response")?;
    let pid = v.pid.map(|p| format!(" (PID {p})")).unwrap_or_default();
    println!("{}: {}{}", v.spec.id, v.state, pid);
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn dates() {
        assert_eq!(super::time(0), "1970-01-01 00:00:00Z");
        assert_eq!(super::time(951_782_400_000), "2000-02-29 00:00:00Z");
    }
}
