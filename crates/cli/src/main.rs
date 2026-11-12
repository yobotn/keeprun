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
    /// Show agent status and the active environment.
    Status,
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
        app: String,
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
    },
    /// Show, set or remove environment variables.
    Env {
        app: String,
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
    Remove {
        app: String,
    },
    Start {
        app: String,
    },
    Stop {
        app: String,
    },
    Restart {
        app: String,
    },
    /// Show app logs.
    Logs {
        app: String,
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
        app: String,
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
        app: String,
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
    Run {
        app: String,
        /// Saved task name. See `keeprun tasks <app>`.
        task: Option<String>,
        /// Start the task and return without waiting for it.
        #[arg(long)]
        no_wait: bool,
        #[arg(last = true)]
        command: Vec<String>,
    },
    /// Run an MCP server over stdio, so coding agents check status, read logs and restart
    /// apps with typed tools. Tools default to the app in the current folder.
    ///
    /// Add it to Claude Code:
    ///   claude mcp add keeprun -- keeprun mcp
    Mcp {
        /// Only read status, logs, errors and events. No start, stop or restart.
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

    fn unreachable(&self) -> anyhow::Error {
        anyhow::anyhow!(
            "Cannot reach the {} agent at {}. Start it with `keeprun-agent run` or start the service.",
            self.settings.env,
            self.settings.base_url()
        )
    }
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
        (s, _) if a.issue.is_some() => format!("{s}, errors"),
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

    match cli.cmd {
        Cmd::Status => {
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
            let dir = dir.map_or_else(std::env::current_dir, Ok)?;
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
        Cmd::Status | Cmd::Detect { .. } => unreachable!(),
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
        } => {
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
                ..Default::default()
            };
            let v: AppView = c.send(Method::PATCH, &format!("/apps/{app}"), &u).await?;
            println!("Saved {}.", v.spec.id);
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
            let v: AppView = c.get(&format!("/apps/{app}")).await?;
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
        Cmd::Start { app } => action(&c, &app, "start").await?,
        Cmd::Stop { app } => action(&c, &app, "stop").await?,
        Cmd::Restart { app } => action(&c, &app, "restart").await?,
        Cmd::Issue { app, clear } => {
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
            let v: AppView = c.get(&format!("/apps/{app}")).await?;
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
        Cmd::Run {
            app,
            task,
            no_wait,
            command,
        } => {
            let body = match (task, command.is_empty()) {
                (Some(name), true) => RunTask {
                    name: Some(name),
                    command: None,
                },
                (None, false) => RunTask {
                    name: None,
                    command: Some(command.join(" ")),
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

async fn action(c: &Client, app: &str, verb: &str) -> Result<()> {
    let v: AppView = c
        .call(Method::POST, &format!("/apps/{app}/{verb}"), None::<&()>)
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
