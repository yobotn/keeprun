use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

/// What to do when the app process ends without a Stop request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy {
    /// Leave it stopped.
    Never,
    /// Restart when the exit code is not 0.
    #[default]
    OnFailure,
    /// Restart on any exit.
    Always,
}

impl RestartPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            RestartPolicy::Never => "never",
            RestartPolicy::OnFailure => "on-failure",
            RestartPolicy::Always => "always",
        }
    }
}

impl std::str::FromStr for RestartPolicy {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        match s {
            "never" => Ok(RestartPolicy::Never),
            "on-failure" => Ok(RestartPolicy::OnFailure),
            "always" => Ok(RestartPolicy::Always),
            o => Err(format!(
                "Restart policy \"{o}\" is not valid. Use never, on-failure or always."
            )),
        }
    }
}

fn yes() -> bool {
    true
}
fn five() -> u32 {
    5
}

/// A saved app definition.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppSpec {
    pub id: String,
    pub name: String,
    pub cwd: PathBuf,
    /// Command line run through `cmd.exe /d /s /c`.
    pub command: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// Keys in `env` whose values are secret. Stored encrypted, masked in the UI.
    #[serde(default)]
    pub secrets: BTreeSet<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    /// Start when the agent starts (on boot), unless the user stopped it.
    #[serde(default = "yes")]
    pub autostart: bool,
    #[serde(default)]
    pub restart: RestartPolicy,
    /// Restarts in a row before the app is marked Failed. 0 means no limit.
    #[serde(default = "five")]
    pub max_restarts: u32,
    /// One-off commands run on demand, e.g. a database migration.
    #[serde(default)]
    pub tasks: Vec<TaskSpec>,
    /// Kept but switched off: cannot start, never autostarts. The UI lists it apart.
    #[serde(default)]
    pub disabled: bool,
    /// Watch the output for errors while the app runs, and report them as an issue.
    #[serde(default = "yes")]
    pub watch_errors: bool,
    /// Env file read on every start, e.g. ".env.local". Relative to `cwd`, or absolute.
    /// Variables in `env` override the file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_file: Option<String>,
}

/// A saved one-off command for an app. Runs in the app folder with the app environment.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskSpec {
    pub name: String,
    pub command: String,
}

/// Body of `POST /apps/{id}/tasks/run`: a saved task name or a command line.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RunTask {
    pub name: Option<String>,
    pub command: Option<String>,
}

/// The running or most recent task of an app.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskRun {
    /// Saved task name, or none for a one-off command.
    pub name: Option<String>,
    pub command: String,
    pub running: bool,
    /// Unix time in milliseconds.
    pub started_at: u64,
    pub finished_at: Option<u64>,
    pub exit_code: Option<i32>,
}

/// Body of `POST /apps`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewApp {
    pub name: String,
    pub cwd: PathBuf,
    pub command: String,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    #[serde(default)]
    pub secrets: BTreeSet<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default = "yes")]
    pub autostart: bool,
    #[serde(default)]
    pub restart: RestartPolicy,
    #[serde(default = "five")]
    pub max_restarts: u32,
    #[serde(default)]
    pub tasks: Vec<TaskSpec>,
    #[serde(default = "yes")]
    pub watch_errors: bool,
    #[serde(default)]
    pub env_file: Option<String>,
}

/// Body of `PATCH /apps/{id}`. Missing fields keep their value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateApp {
    pub name: Option<String>,
    pub cwd: Option<PathBuf>,
    pub command: Option<String>,
    pub env: Option<BTreeMap<String, String>>,
    pub secrets: Option<BTreeSet<String>>,
    pub tags: Option<Vec<String>>,
    pub autostart: Option<bool>,
    pub restart: Option<RestartPolicy>,
    pub max_restarts: Option<u32>,
    pub tasks: Option<Vec<TaskSpec>>,
    pub disabled: Option<bool>,
    pub watch_errors: Option<bool>,
    /// Env file to read on every start. An empty string unlinks it.
    pub env_file: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AppState {
    Stopped,
    Starting,
    Running,
    Stopping,
    /// Ended and not restarted (policy says no).
    Exited,
    /// Waiting for the backoff delay before the next restart.
    Restarting,
    /// Crashed more than `max_restarts` times in a row.
    Failed,
}

impl AppState {
    /// A process exists or will exist soon.
    pub fn is_active(self) -> bool {
        matches!(
            self,
            AppState::Starting | AppState::Running | AppState::Stopping | AppState::Restarting
        )
    }
}

impl std::fmt::Display for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            AppState::Stopped => "Stopped",
            AppState::Starting => "Starting",
            AppState::Running => "Running",
            AppState::Stopping => "Stopping",
            AppState::Exited => "Exited",
            AppState::Restarting => "Restarting",
            AppState::Failed => "Failed",
        };
        f.write_str(s)
    }
}

/// Live resource use of the whole process tree.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct Stats {
    pub cpu_percent: f32,
    pub memory_bytes: u64,
    pub processes: u32,
    /// TCP ports the tree is listening on.
    pub ports: Vec<u16>,
}

/// An app plus its live status, as returned by `GET /apps`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppView {
    #[serde(flatten)]
    pub spec: AppSpec,
    pub state: AppState,
    pub pid: Option<u32>,
    pub exit_code: Option<i32>,
    /// Unix time in milliseconds.
    pub started_at: Option<u64>,
    /// Restarts in a row since the last stable run.
    #[serde(default)]
    pub restarts: u32,
    /// Unix time in milliseconds of the next restart attempt.
    pub next_restart_at: Option<u64>,
    pub stats: Option<Stats>,
    /// The running or most recent task since the agent started.
    #[serde(default)]
    pub task: Option<TaskRun>,
    /// Errors in the output of the current run. Only while `watch_errors` is on.
    #[serde(default)]
    pub issue: Option<AppIssue>,
    /// State of the linked env file. None when the app has no env file.
    #[serde(default)]
    pub env_file_status: Option<EnvFileStatus>,
}

/// The linked env file as the agent sees it. Keys only: values may be secret.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct EnvFileStatus {
    /// Keys in the file now.
    pub keys: Vec<String>,
    /// PORT in the file, if any.
    pub port: Option<u16>,
    /// Keys added, removed or changed since the running process started. Empty when the
    /// app is not running or nothing changed.
    pub changed: Vec<String>,
    /// Why the file cannot be read, e.g. it was deleted.
    pub error: Option<String>,
}

/// Errors an app printed while it kept running, e.g. a dev server whose build broke.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AppIssue {
    /// Unix time in milliseconds.
    pub first_at: u64,
    pub last_at: u64,
    /// Times the first error was printed.
    pub count: u32,
    /// The first error line.
    pub summary: String,
    /// Output around the first occurrence, with repeats removed.
    pub excerpt: Vec<String>,
}

/// Body of `GET /apps/{id}/issue`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueReport {
    pub issue: Option<AppIssue>,
    /// Text for a coding agent, when there is an issue.
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EventKind {
    Added,
    ConfigChanged,
    Started,
    StartFailed,
    Stopped,
    Exited,
    Crashed,
    Restarted,
    Failed,
    TaskStarted,
    TaskFinished,
    TaskFailed,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            EventKind::Added => "added",
            EventKind::ConfigChanged => "config-changed",
            EventKind::Started => "started",
            EventKind::StartFailed => "start-failed",
            EventKind::Stopped => "stopped",
            EventKind::Exited => "exited",
            EventKind::Crashed => "crashed",
            EventKind::Restarted => "restarted",
            EventKind::Failed => "failed",
            EventKind::TaskStarted => "task-started",
            EventKind::TaskFinished => "task-finished",
            EventKind::TaskFailed => "task-failed",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "added" => EventKind::Added,
            "config-changed" => EventKind::ConfigChanged,
            "started" => EventKind::Started,
            "start-failed" => EventKind::StartFailed,
            "stopped" => EventKind::Stopped,
            "exited" => EventKind::Exited,
            "crashed" => EventKind::Crashed,
            "restarted" => EventKind::Restarted,
            "failed" => EventKind::Failed,
            "task-started" => EventKind::TaskStarted,
            "task-finished" => EventKind::TaskFinished,
            "task-failed" => EventKind::TaskFailed,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub id: i64,
    pub app_id: String,
    /// Unix time in milliseconds.
    pub ts: u64,
    pub kind: EventKind,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    Stdout,
    Stderr,
    System,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogLine {
    /// Unix time in milliseconds.
    pub ts: u64,
    pub stream: Stream,
    pub text: String,
    /// Output of a task rather than the app itself.
    #[serde(default, skip_serializing_if = "is_false")]
    pub task: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Response of `GET /health`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Health {
    pub ok: bool,
    pub env: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiError {
    pub error: String,
}

/// Format a Unix millisecond timestamp as UTC, like `2026-11-11 18:58:25Z`.
pub fn format_utc(ts: u64) -> String {
    let secs = ts / 1000;
    let (h, m, s) = ((secs / 3600) % 24, (secs / 60) % 60, secs % 60);
    let (y, mo, d) = civil_from_days((secs / 86400) as i64);
    format!("{y:04}-{mo:02}-{d:02} {h:02}:{m:02}:{s:02}Z")
}

/// Days since 1970-01-01 to (year, month, day). Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// Turn a display name into a URL safe id: "HR Portal" becomes "hr-portal".
pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.trim().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

/// Parse `.env` text into key/value pairs. Supports comments, `export`, and quoted values.
pub fn parse_dotenv(text: &str) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    for (i, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").unwrap_or(line);
        let Some((k, v)) = line.split_once('=') else {
            return Err(format!("Line {} is not KEY=VALUE: {line}", i + 1));
        };
        let k = k.trim();
        if k.is_empty()
            || !k
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-')
        {
            return Err(format!("Line {} has an invalid key: {k}", i + 1));
        }
        let v = v.trim();
        let v = if v.len() >= 2
            && ((v.starts_with('"') && v.ends_with('"'))
                || (v.starts_with('\'') && v.ends_with('\'')))
        {
            v[1..v.len() - 1].to_string()
        } else {
            // Strip a trailing comment on unquoted values.
            v.split(" #").next().unwrap_or("").trim().to_string()
        };
        out.insert(k.to_string(), v);
    }
    Ok(out)
}

/// Read and parse an app's linked env file. `file` is relative to `cwd`, or absolute.
/// A UTF-8 byte order mark is dropped: Windows editors often write one.
pub fn read_env_file(cwd: &Path, file: &str) -> Result<BTreeMap<String, String>, String> {
    let path = cwd.join(file);
    let text = std::fs::read_to_string(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            format!("{file} does not exist in {}.", cwd.display())
        } else {
            format!("Cannot read {file}: {e}")
        }
    })?;
    parse_dotenv(text.strip_prefix('\u{feff}').unwrap_or(&text)).map_err(|e| format!("{file}: {e}"))
}

/// Keys whose value differs between two env maps, including added and removed keys.
pub fn changed_keys(a: &BTreeMap<String, String>, b: &BTreeMap<String, String>) -> Vec<String> {
    let mut keys: Vec<String> = a
        .keys()
        .chain(b.keys())
        .filter(|k| a.get(*k) != b.get(*k))
        .cloned()
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// Key names that usually hold secrets. Used to pre-mark imported values.
pub fn looks_secret(key: &str) -> bool {
    let k = key.to_ascii_uppercase();
    [
        "SECRET",
        "PASSWORD",
        "PASSWD",
        "TOKEN",
        "API_KEY",
        "APIKEY",
        "PRIVATE",
        "CONNECTION_STRING",
        "CREDENTIAL",
    ]
    .iter()
    .any(|w| k.contains(w))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_names() {
        assert_eq!(slugify("HR Portal"), "hr-portal");
        assert_eq!(slugify("  sales_api v2 "), "sales-api-v2");
        assert_eq!(slugify("!!!"), "");
    }

    #[test]
    fn dotenv() {
        let env = parse_dotenv("# c\nPORT=3000\nexport A = \"x y\"\nB='q' \nC=1 # note\nEMPTY=\n")
            .unwrap();
        assert_eq!(env["PORT"], "3000");
        assert_eq!(env["A"], "x y");
        assert_eq!(env["B"], "q");
        assert_eq!(env["C"], "1");
        assert_eq!(env["EMPTY"], "");
        assert!(parse_dotenv("nope").is_err());
    }

    #[test]
    fn secrets() {
        assert!(looks_secret("DB_PASSWORD"));
        assert!(looks_secret("stripe_api_key"));
        assert!(!looks_secret("PORT"));
    }

    #[test]
    fn spec_defaults() {
        let s: AppSpec =
            serde_json::from_str(r#"{"id":"a","name":"a","cwd":"C:\\","command":"x"}"#).unwrap();
        assert!(s.autostart);
        assert_eq!(s.restart, RestartPolicy::OnFailure);
        assert_eq!(s.max_restarts, 5);
        assert!(s.watch_errors);
    }

    #[test]
    fn utc() {
        assert_eq!(format_utc(0), "1970-01-01 00:00:00Z");
        assert_eq!(format_utc(1_709_210_096_000), "2024-02-29 12:34:56Z");
    }
}
