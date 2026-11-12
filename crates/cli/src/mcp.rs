//! `keeprun mcp`: a Model Context Protocol server over stdio, so coding agents and MCP clients
//! check status, read logs and restart apps with typed tools instead of a shell.
//!
//! JSON-RPC 2.0, one message per line. It uses the agent's local API with the user's token,
//! so there is no new port or service. Environment values are never returned, only keys.
//! Tools that take an app default to the app whose folder holds the current folder: an agent
//! started in a project folder names nothing.

use crate::Client;
use anyhow::{anyhow, bail, Context, Result};
use keeprun_core::model::{AppState, AppView, Event, IssueReport, LogLine, RunTask, Stream};
use reqwest::Method;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// Protocol versions this server speaks, newest first.
const VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];
const MAX_LOG_LINES: usize = 2000;

/// What the agent may do through this server.
#[derive(Clone, Copy)]
pub struct Access {
    /// Start, stop and restart apps.
    pub control: bool,
    /// Run saved tasks.
    pub tasks: bool,
}

pub async fn serve(client: Client, access: Access) -> Result<()> {
    let server = Server {
        client,
        access,
        cwd: std::env::current_dir().unwrap_or_default(),
    };
    let mut lines = BufReader::new(tokio::io::stdin()).lines();
    let mut out = tokio::io::stdout();
    while let Some(line) = lines.next_line().await? {
        if line.trim().is_empty() {
            continue;
        }
        let reply = match serde_json::from_str::<Value>(&line) {
            Ok(Value::Array(batch)) => {
                let mut replies = vec![];
                for msg in batch {
                    if let Some(r) = server.handle(msg).await {
                        replies.push(r);
                    }
                }
                (!replies.is_empty()).then_some(Value::Array(replies))
            }
            Ok(msg) => server.handle(msg).await,
            Err(e) => Some(error(Value::Null, -32700, &format!("Parse error: {e}"))),
        };
        if let Some(r) = reply {
            out.write_all(format!("{r}\n").as_bytes()).await?;
            out.flush().await?;
        }
    }
    Ok(())
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

struct Server {
    client: Client,
    access: Access,
    cwd: PathBuf,
}

impl Server {
    /// Answer one message. Notifications get no answer.
    async fn handle(&self, msg: Value) -> Option<Value> {
        let id = msg.get("id").cloned();
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        let id = id?;
        let result = match method {
            "initialize" => Ok(self.initialize(&params)),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": self.tools() })),
            "tools/call" => Ok(self.call(&params).await),
            "resources/list" => Ok(json!({ "resources": [] })),
            "prompts/list" => Ok(json!({ "prompts": [] })),
            _ => Err(error(
                id.clone(),
                -32601,
                &format!("Method not found: {method}"),
            )),
        };
        Some(match result {
            Ok(r) => json!({ "jsonrpc": "2.0", "id": id, "result": r }),
            Err(e) => e,
        })
    }

    fn initialize(&self, params: &Value) -> Value {
        let asked = params.get("protocolVersion").and_then(Value::as_str);
        let version = asked
            .filter(|v| VERSIONS.contains(v))
            .unwrap_or(VERSIONS[0]);
        json!({
            "protocolVersion": version,
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "keeprun", "version": env!("CARGO_PKG_VERSION") },
            "instructions": "Keeprun runs the user's apps (dev servers, APIs) on Windows. \
                Do not start an app Keeprun runs yourself (npm run dev and similar): a second \
                copy fights over the port and its errors never reach Keeprun. Read its logs \
                and errors here, and restart it here after changing config or dependencies. \
                Tools default to the app in the current folder."
        })
    }

    fn tools(&self) -> Vec<Value> {
        let app = json!({
            "type": "string",
            "description": "App id from list_apps. Defaults to the app whose folder holds the current folder."
        });
        let read_only = json!({ "readOnlyHint": true, "openWorldHint": false });
        let mut tools = vec![
            json!({
                "name": "list_apps",
                "title": "List apps",
                "description": "Every app Keeprun runs, with state, URL, ports, folder and command, and any errors it is printing.",
                "inputSchema": { "type": "object", "properties": {} },
                "annotations": read_only,
            }),
            json!({
                "name": "app_status",
                "title": "App status",
                "description": "State, URL, ports, CPU, memory, uptime, restarts and current errors of one app, plus copies running outside Keeprun.",
                "inputSchema": { "type": "object", "properties": { "app": app } },
                "annotations": read_only,
            }),
            json!({
                "name": "read_logs",
                "title": "Read logs",
                "description": "Recent output of an app, oldest first. stderr lines are marked ERR, Keeprun's own lines SYS.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "app": app,
                        "tail": { "type": "integer", "minimum": 1, "maximum": MAX_LOG_LINES, "default": 200, "description": "Number of recent lines." },
                        "errors_only": { "type": "boolean", "default": false, "description": "Only stderr lines." },
                        "search": { "type": "string", "description": "Only lines that contain this text, ignoring case." }
                    }
                },
                "annotations": read_only,
            }),
            json!({
                "name": "get_errors",
                "title": "Get errors",
                "description": "Errors the app printed while running, with the output around them and the folder and commands to fix and check it. Empty when there are none.",
                "inputSchema": { "type": "object", "properties": { "app": app } },
                "annotations": read_only,
            }),
            json!({
                "name": "list_events",
                "title": "List events",
                "description": "Recent events, newest first: started, stopped, crashed, restarted, config changed.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "app": { "type": "string", "description": "App id. Leave out for every app." },
                        "limit": { "type": "integer", "minimum": 1, "maximum": 500, "default": 30 }
                    }
                },
                "annotations": read_only,
            }),
        ];
        if self.access.control {
            for (name, title, description) in [
                ("start_app", "Start app", "Start an app and wait until it runs. Copies of it running outside Keeprun are ended first."),
                ("stop_app", "Stop app", "Stop an app and its whole process tree. It stays stopped until started again."),
                ("restart_app", "Restart app", "Restart an app, e.g. after changing config, env files or dependencies, and wait until it runs. Returns its state and the first output of the new run."),
            ] {
                tools.push(json!({
                    "name": name,
                    "title": title,
                    "description": description,
                    "inputSchema": { "type": "object", "properties": { "app": app } },
                    "annotations": { "destructiveHint": name == "stop_app", "idempotentHint": name != "restart_app", "openWorldHint": false },
                }));
            }
        }
        if self.access.tasks {
            tools.push(json!({
                "name": "run_task",
                "title": "Run task",
                "description": "Run a saved task of an app (e.g. build, migrate, test) and return its output once it ends.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "app": app,
                        "task": { "type": "string", "description": "Saved task name. app_status lists them." }
                    },
                    "required": ["task"]
                },
                "annotations": { "destructiveHint": true, "openWorldHint": false },
            }));
        }
        tools
    }

    async fn call(&self, params: &Value) -> Value {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let args = params.get("arguments").cloned().unwrap_or(json!({}));
        match self.run(name, &args).await {
            Ok(text) => json!({ "content": [{ "type": "text", "text": text }] }),
            Err(e) => {
                json!({ "content": [{ "type": "text", "text": format!("{e:#}") }], "isError": true })
            }
        }
    }

    async fn run(&self, name: &str, args: &Value) -> Result<String> {
        let control = matches!(name, "start_app" | "stop_app" | "restart_app");
        if control && !self.access.control {
            bail!("This Keeprun MCP server is read-only. Start it without --read-only to allow {name}.");
        }
        if name == "run_task" && !self.access.tasks {
            bail!("Running tasks is off. Start the server with --allow-tasks to allow it.");
        }
        match name {
            "list_apps" => {
                let apps: Vec<AppView> = self.client.get("/apps").await?;
                if apps.is_empty() {
                    return Ok("No apps yet. Add one in the Keeprun app or with `keeprun add --cwd <folder>`.".into());
                }
                let here = owner(&apps, &self.cwd).map(|a| a.spec.id.clone());
                Ok(to_text(&json!(apps
                    .iter()
                    .map(|a| {
                        let mut v = summary(a);
                        if Some(&a.spec.id) == here.as_ref() {
                            v["current_folder"] = json!(true);
                        }
                        v
                    })
                    .collect::<Vec<_>>())))
            }
            "app_status" => {
                let a = self.app(args).await?;
                Ok(to_text(&detail(&a)))
            }
            "read_logs" => {
                let a = self.app(args).await?;
                let tail = args
                    .get("tail")
                    .and_then(Value::as_u64)
                    .unwrap_or(200)
                    .clamp(1, MAX_LOG_LINES as u64) as usize;
                let errors_only = args
                    .get("errors_only")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let search = args
                    .get("search")
                    .and_then(Value::as_str)
                    .map(str::to_lowercase)
                    .filter(|s| !s.is_empty());
                // Filters apply to the whole buffer, then the tail is taken.
                let fetch = if errors_only || search.is_some() {
                    MAX_LOG_LINES
                } else {
                    tail
                };
                let lines: Vec<LogLine> = self
                    .client
                    .get(&format!(
                        "/apps/{}/logs/recent?tail={fetch}",
                        enc(&a.spec.id)
                    ))
                    .await?;
                let picked: Vec<&LogLine> = lines
                    .iter()
                    .filter(|l| !errors_only || l.stream == Stream::Stderr)
                    .filter(|l| {
                        search
                            .as_ref()
                            .is_none_or(|q| l.text.to_lowercase().contains(q))
                    })
                    .collect();
                let picked = &picked[picked.len().saturating_sub(tail)..];
                if picked.is_empty() {
                    return Ok(format!(
                        "No matching output from {} ({}).",
                        a.spec.id,
                        state(&a)
                    ));
                }
                Ok(picked
                    .iter()
                    .map(|l| log_line(l))
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "get_errors" => {
                let a = self.app(args).await?;
                let r: IssueReport = self
                    .client
                    .get(&format!("/apps/{}/issue", enc(&a.spec.id)))
                    .await?;
                Ok(r.prompt.unwrap_or_else(|| {
                    format!(
                        "{} is {} and printed no errors in this run.",
                        a.spec.id,
                        state(&a)
                    )
                }))
            }
            "list_events" => {
                let limit = args
                    .get("limit")
                    .and_then(Value::as_u64)
                    .unwrap_or(30)
                    .clamp(1, 500);
                let path = match args.get("app").and_then(Value::as_str) {
                    Some(id) => format!("/apps/{}/events?limit={limit}", enc(id)),
                    None => format!("/events?limit={limit}"),
                };
                let events: Vec<Event> = self.client.get(&path).await?;
                if events.is_empty() {
                    return Ok("No events yet.".into());
                }
                Ok(events
                    .iter()
                    .map(|e| {
                        format!(
                            "{}  {:<8}  {:?}  {}",
                            time(e.ts),
                            e.app_id,
                            e.kind,
                            e.message
                        )
                    })
                    .collect::<Vec<_>>()
                    .join("\n"))
            }
            "start_app" | "restart_app" => {
                let a = self.app(args).await?;
                let verb = if name == "start_app" {
                    "start"
                } else {
                    "restart"
                };
                let since = crate::now_ms();
                let _: AppView = self
                    .client
                    .send(
                        Method::POST,
                        &format!("/apps/{}/{verb}", enc(&a.spec.id)),
                        &json!({}),
                    )
                    .await?;
                let a = self.settle(&a.spec.id).await?;
                let lines: Vec<LogLine> = self
                    .client
                    .get(&format!("/apps/{}/logs/recent?tail=40", enc(&a.spec.id)))
                    .await?;
                // From the new run's start line on: a restart first logs the old run stopping.
                let recent: Vec<&LogLine> = lines.iter().filter(|l| l.ts >= since).collect();
                let from = recent
                    .iter()
                    .rposition(|l| l.stream == Stream::System && l.text.starts_with("Started:"))
                    .unwrap_or(0);
                let new: Vec<String> = recent[from..].iter().map(|l| log_line(l)).collect();
                let mut text = to_text(&detail(&a));
                if !new.is_empty() {
                    text.push_str("\n\nOutput of the new run:\n");
                    text.push_str(&new.join("\n"));
                }
                Ok(text)
            }
            "stop_app" => {
                let a = self.app(args).await?;
                let a: AppView = self
                    .client
                    .send(
                        Method::POST,
                        &format!("/apps/{}/stop", enc(&a.spec.id)),
                        &json!({}),
                    )
                    .await?;
                Ok(format!("{} is {}.", a.spec.id, state(&a)))
            }
            "run_task" => {
                let a = self.app(args).await?;
                let task = args
                    .get("task")
                    .and_then(Value::as_str)
                    .context("Give the task name.")?;
                if !a.spec.tasks.iter().any(|t| t.name == task) {
                    let names: Vec<&str> = a.spec.tasks.iter().map(|t| t.name.as_str()).collect();
                    bail!(
                        "{} has no task \"{task}\". Saved tasks: {}.",
                        a.spec.id,
                        if names.is_empty() {
                            "none".into()
                        } else {
                            names.join(", ")
                        }
                    );
                }
                let since = crate::now_ms();
                let body = RunTask {
                    name: Some(task.into()),
                    command: None,
                };
                let _: AppView = self
                    .client
                    .send(
                        Method::POST,
                        &format!("/apps/{}/tasks/run", enc(&a.spec.id)),
                        &body,
                    )
                    .await?;
                // Wait for the task to end, up to 10 minutes.
                let mut run = None;
                for _ in 0..1200 {
                    tokio::time::sleep(Duration::from_millis(500)).await;
                    let v: AppView = self
                        .client
                        .get(&format!("/apps/{}", enc(&a.spec.id)))
                        .await?;
                    if let Some(t) = v.task.filter(|t| t.started_at >= since && !t.running) {
                        run = Some(t);
                        break;
                    }
                }
                let lines: Vec<LogLine> = self
                    .client
                    .get(&format!(
                        "/apps/{}/logs/recent?tail={MAX_LOG_LINES}",
                        enc(&a.spec.id)
                    ))
                    .await?;
                let output: Vec<String> = lines
                    .iter()
                    .filter(|l| l.task && l.ts >= since)
                    .map(log_line)
                    .collect();
                let head = match &run {
                    Some(t) => match t.exit_code {
                        Some(0) => format!("Task \"{task}\" succeeded."),
                        Some(c) => format!("Task \"{task}\" failed with exit code {c}."),
                        None => format!("Task \"{task}\" ended."),
                    },
                    None => format!("Task \"{task}\" is still running after 10 minutes. Read its output later with read_logs."),
                };
                Ok(format!("{head}\n\n{}", output.join("\n")))
            }
            _ => bail!("Unknown tool \"{name}\"."),
        }
    }

    /// The app named in `args`, or the one whose folder holds the current folder.
    async fn app(&self, args: &Value) -> Result<AppView> {
        if let Some(id) = args
            .get("app")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        {
            return self.client.get(&format!("/apps/{}", enc(id))).await;
        }
        let apps: Vec<AppView> = self.client.get("/apps").await?;
        owner(&apps, &self.cwd).cloned().ok_or_else(|| {
            let ids: Vec<&str> = apps.iter().map(|a| a.spec.id.as_str()).collect();
            anyhow!(
                "No app runs from {}. Name one with \"app\": {}.",
                self.cwd.display(),
                if ids.is_empty() {
                    "there are no apps yet".into()
                } else {
                    ids.join(", ")
                }
            )
        })
    }

    /// Wait up to 20 seconds for a start to settle: running with a port, or not active.
    async fn settle(&self, id: &str) -> Result<AppView> {
        let mut last = None;
        for i in 0..40 {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let a: AppView = self.client.get(&format!("/apps/{}", enc(id))).await?;
            let listening = a.stats.as_ref().is_some_and(|s| !s.ports.is_empty());
            let done = match a.state {
                AppState::Running => listening || a.issue.is_some() || i >= 20,
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
}

/// The app whose folder is the current folder or holds it. The deepest wins.
fn owner<'a>(apps: &'a [AppView], cwd: &Path) -> Option<&'a AppView> {
    let key = |p: &Path| {
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

fn enc(id: &str) -> String {
    id.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn time(ts: u64) -> String {
    keeprun_core::model::format_utc(ts)
}

fn log_line(l: &LogLine) -> String {
    let tag = match l.stream {
        Stream::Stdout => "   ",
        Stream::Stderr => "ERR",
        Stream::System => "SYS",
    };
    let task = if l.task { " task" } else { "" };
    format!("{} {tag}{task} {}", time(l.ts), l.text)
}

fn to_text(v: &Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_default()
}

/// State in lowercase, as in "web is running".
fn state(a: &AppView) -> String {
    if a.spec.disabled {
        "disabled".into()
    } else {
        a.state.to_string().to_lowercase()
    }
}

fn url(a: &AppView) -> Option<String> {
    a.stats
        .as_ref()
        .and_then(|s| s.ports.first())
        .map(|p| format!("http://localhost:{p}"))
}

/// One app for list_apps. Environment values are left out: they may be secret.
fn summary(a: &AppView) -> Value {
    let mut v = json!({
        "id": a.spec.id,
        "name": a.spec.name,
        "state": state(a),
        "folder": a.spec.cwd,
        "command": a.spec.command,
    });
    if let Some(u) = url(a) {
        v["url"] = json!(u);
    }
    if let Some(i) = &a.issue {
        v["errors"] = json!(i.summary);
    }
    if let Some(h) = a.health.as_ref().filter(|h| !h.ok) {
        v["unhealthy"] = json!(h.detail);
    }
    if !a.outside.is_empty() {
        v["outside_copies"] = json!(a.outside);
    }
    v
}

fn detail(a: &AppView) -> Value {
    let mut v = summary(a);
    v["pid"] = json!(a.pid);
    v["exit_code"] = json!(a.exit_code);
    v["restarts"] = json!(a.restarts);
    if let Some(t) = a.started_at {
        v["started_at"] = json!(time(t));
    }
    if let Some(s) = &a.stats {
        v["ports"] = json!(s.ports);
        v["cpu_percent"] = json!((s.cpu_percent as f64 * 10.0).round() / 10.0);
        v["memory_mb"] = json!(s.memory_bytes / 1024 / 1024);
        v["processes"] = json!(s.processes);
    }
    if let Some(i) = &a.issue {
        v["errors"] = json!({ "first": i.summary, "count": i.count, "last_at": time(i.last_at), "output": i.excerpt });
    }
    if let Some(check) = &a.spec.health_check {
        v["health_check"] = json!(check);
    }
    if let Some(h) = &a.health {
        v["health"] = json!({ "ok": h.ok, "detail": h.detail, "failures_in_a_row": h.failures, "checked_at": time(h.checked_at) });
    }
    v["env_keys"] = json!(a.spec.env.keys().collect::<Vec<_>>());
    if let Some(f) = &a.spec.env_file {
        v["env_file"] = json!(f);
    }
    if !a.spec.tasks.is_empty() {
        v["tasks"] = json!(a
            .spec
            .tasks
            .iter()
            .map(|t| json!({ "name": t.name, "command": t.command }))
            .collect::<Vec<_>>());
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(id: &str, cwd: &str) -> AppView {
        serde_json::from_value(json!({
            "id": id, "name": id, "cwd": cwd, "command": "npm run dev",
            "env": { "API_KEY": "secret-value" }, "secrets": ["API_KEY"],
            "state": "running", "pid": 1, "exit_code": null, "started_at": null,
            "next_restart_at": null, "stats": { "cpu_percent": 1.0, "memory_bytes": 1, "processes": 1, "ports": [3000] }
        }))
        .unwrap()
    }

    #[test]
    fn picks_the_deepest_app_that_holds_the_folder() {
        let apps = [
            app("repo", r"C:\Code\Repo"),
            app("web", r"C:\code\repo\apps\web"),
        ];
        let id = |p: &str| owner(&apps, Path::new(p)).map(|a| a.spec.id.as_str());
        assert_eq!(id(r"C:\code\repo\apps\web\src"), Some("web"));
        assert_eq!(id(r"c:\CODE\repo"), Some("repo"));
        assert_eq!(id(r"C:\code\repository"), None);
    }

    #[test]
    fn never_returns_env_values() {
        let text = to_text(&detail(&app("web", r"C:\web")));
        assert!(text.contains("API_KEY"));
        assert!(!text.contains("secret-value"));
        assert!(text.contains("http://localhost:3000"));
    }

    #[test]
    fn encodes_app_ids_for_paths() {
        assert_eq!(enc("hr portal/x"), "hr%20portal%2Fx");
        assert_eq!(enc("web-1.2_a"), "web-1.2_a");
    }
}
