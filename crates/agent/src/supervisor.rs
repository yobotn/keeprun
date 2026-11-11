//! Owns every managed process: start, stop, restart policy, stats, logs and events.
//!
//! Desired state works like Docker's "unless-stopped": an app the user started is started
//! again when the agent starts (boot, crash, upgrade) if `autostart` is on. An app the user
//! stopped stays stopped. Agent shutdown does not change desired state.

use crate::db::Db;
use crate::job::{self, Job};
use anyhow::{anyhow, Context, Result};
use keeprun_core::issue::{self, IssueTracker};
use keeprun_core::model::{
    changed_keys, read_env_file, slugify, AppSpec, AppState, AppView, EnvFileStatus, Event,
    EventKind, IssueReport, LogLine, NewApp, RestartPolicy, RunTask, Stats, Stream, TaskRun,
    TaskSpec, UpdateApp,
};
use keeprun_core::Settings;
use std::collections::{BTreeMap, HashSet, VecDeque};
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use tokio::io::{AsyncBufReadExt, AsyncRead, BufReader};
use tokio::sync::broadcast;

const LOG_BUFFER_LINES: usize = 2000;
const LOG_FILE_MAX_BYTES: u64 = 10 * 1024 * 1024;
/// A run longer than this resets the restart counter.
const STABLE_RUN: Duration = Duration::from_secs(60);
const MAX_BACKOFF: Duration = Duration::from_secs(60);

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Delay before restart number `n` (1 based): 1s, 2s, 4s ... capped at 60s.
pub fn backoff(n: u32) -> Duration {
    let secs = 1u64.checked_shl(n.saturating_sub(1)).unwrap_or(u64::MAX);
    Duration::from_secs(secs).min(MAX_BACKOFF)
}

/// Error kinds the API maps to HTTP status codes.
#[derive(Debug)]
pub enum SupError {
    NotFound(String),
    Conflict(String),
    BadRequest(String),
    Internal(anyhow::Error),
}

impl From<anyhow::Error> for SupError {
    fn from(e: anyhow::Error) -> Self {
        SupError::Internal(e)
    }
}

impl std::fmt::Display for SupError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SupError::NotFound(s) | SupError::Conflict(s) | SupError::BadRequest(s) => {
                f.write_str(s)
            }
            SupError::Internal(e) => write!(f, "{e:#}"),
        }
    }
}

pub struct Supervisor {
    settings: Arc<Settings>,
    db: Db,
    apps: RwLock<BTreeMap<String, Arc<Managed>>>,
    ncpu: f32,
}

struct Managed {
    spec: RwLock<AppSpec>,
    rt: Mutex<Runtime>,
    logs: LogHub,
    task: Mutex<TaskSlot>,
    env_cache: Mutex<EnvCache>,
}

type EnvVars = BTreeMap<String, String>;

/// The linked env file, parsed again only when its path, size or modified time changes.
/// Status polls ask for it every second.
#[derive(Default)]
struct EnvCache {
    stamp: Option<(PathBuf, Option<SystemTime>, Option<u64>)>,
    vars: Option<Result<EnvVars, String>>,
}

/// One task at a time per app. Tasks run beside the app and are never restarted.
#[derive(Default)]
struct TaskSlot {
    run: Option<TaskRun>,
    job: Option<Arc<Job>>,
    /// Increments per task, so a late exit never overwrites a newer run.
    seq: u64,
    stop_requested: bool,
}

#[derive(Default)]
struct Runtime {
    state: Option<AppState>,
    pid: Option<u32>,
    exit_code: Option<i32>,
    started_at: Option<u64>,
    started_instant: Option<Instant>,
    job: Option<Arc<Job>>,
    /// Increments on every spawn and on cancel, so stale exits and timers are ignored.
    generation: u64,
    restarts: u32,
    next_restart_at: Option<u64>,
    cpu_sample: Option<(Instant, u64)>,
    cpu_percent: f32,
    /// Env file variables the current process started with. None without a linked file or
    /// when it could not be read at start.
    env_file_loaded: Option<EnvVars>,
}

pub struct LogHub {
    tx: broadcast::Sender<LogLine>,
    buffer: Mutex<VecDeque<LogLine>>,
    file: Mutex<Option<(PathBuf, File)>>,
    /// Errors in the app output of the current run. Task output does not count.
    issue: Mutex<IssueTracker>,
}

impl LogHub {
    fn new(dir: PathBuf) -> Self {
        let (tx, _) = broadcast::channel(1024);
        let file = std::fs::create_dir_all(&dir).ok().and_then(|_| {
            let path = dir.join("current.log");
            OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
                .ok()
                .map(|f| (path, f))
        });
        LogHub {
            tx,
            buffer: Mutex::new(VecDeque::with_capacity(LOG_BUFFER_LINES)),
            file: Mutex::new(file),
            issue: Mutex::default(),
        }
    }

    pub fn push(&self, stream: Stream, text: String) {
        self.push_line(stream, text, false);
    }

    fn push_line(&self, stream: Stream, text: String, task: bool) {
        let line = LogLine {
            ts: now_ms(),
            stream,
            text,
            task,
        };
        {
            let mut buf = self.buffer.lock().unwrap();
            if buf.len() == LOG_BUFFER_LINES {
                buf.pop_front();
            }
            buf.push_back(line.clone());
        }
        if !task && stream != Stream::System {
            self.issue.lock().unwrap().feed(&line.text, line.ts);
        }
        self.write_file(&line);
        let _ = self.tx.send(line);
    }

    fn write_file(&self, line: &LogLine) {
        let mut guard = self.file.lock().unwrap();
        let Some((path, file)) = guard.as_mut() else {
            return;
        };
        let tag = match (line.task, line.stream) {
            (false, Stream::Stdout) => "out",
            (false, Stream::Stderr) => "err",
            (false, Stream::System) => "sys",
            (true, Stream::Stdout) => "task-out",
            (true, Stream::Stderr) => "task-err",
            (true, Stream::System) => "task-sys",
        };
        let _ = writeln!(file, "{} {} {}", line.ts, tag, line.text);
        if file
            .metadata()
            .map(|m| m.len() > LOG_FILE_MAX_BYTES)
            .unwrap_or(false)
        {
            let prev = path.with_file_name("previous.log");
            let _ = std::fs::remove_file(&prev);
            let _ = std::fs::rename(&*path, &prev);
            if let Ok(f) = OpenOptions::new().create(true).append(true).open(&*path) {
                *file = f;
            }
        }
    }

    pub fn recent(&self, tail: usize) -> Vec<LogLine> {
        let buf = self.buffer.lock().unwrap();
        buf.iter()
            .skip(buf.len().saturating_sub(tail))
            .cloned()
            .collect()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LogLine> {
        self.tx.subscribe()
    }

    fn clear_issue(&self) {
        self.issue.lock().unwrap().clear();
    }
}

#[cfg(test)]
impl Supervisor {
    fn app_logs_feed_for_test(&self, id: &str, text: &str) {
        self.get(id).unwrap().logs.push(Stream::Stdout, text.into());
    }
}

impl Managed {
    fn state(&self) -> AppState {
        self.rt.lock().unwrap().state.unwrap_or(AppState::Stopped)
    }

    /// The linked env file as it is on disk now. None when the app has no env file.
    fn file_env(&self, spec: &AppSpec) -> Option<Result<EnvVars, String>> {
        let file = spec.env_file.as_deref()?;
        let path = spec.cwd.join(file);
        let meta = std::fs::metadata(&path).ok();
        let stamp = (
            path,
            meta.as_ref().and_then(|m| m.modified().ok()),
            meta.as_ref().map(|m| m.len()),
        );
        let mut cache = self.env_cache.lock().unwrap();
        if cache.stamp.as_ref() != Some(&stamp) || cache.vars.is_none() {
            cache.vars = Some(read_env_file(&spec.cwd, file));
            cache.stamp = Some(stamp);
        }
        cache.vars.clone()
    }

    /// What a process of this app gets: the env file, overridden by the app variables.
    fn effective_env(&self, spec: &AppSpec) -> (EnvVars, Option<Result<EnvVars, String>>) {
        let file = self.file_env(spec);
        let mut env = file.clone().and_then(Result::ok).unwrap_or_default();
        env.extend(spec.env.clone());
        (env, file)
    }

    fn env_file_status(&self, spec: &AppSpec, state: AppState) -> Option<EnvFileStatus> {
        let file = self.file_env(spec)?;
        let loaded = self.rt.lock().unwrap().env_file_loaded.clone();
        let empty = EnvVars::new();
        let now = file.as_ref().unwrap_or(&empty);
        let changed = if state.is_active() {
            changed_keys(loaded.as_ref().unwrap_or(&empty), now)
        } else {
            vec![]
        };
        Some(EnvFileStatus {
            keys: now.keys().cloned().collect(),
            port: job::port_of(now),
            changed,
            error: file.err(),
        })
    }

    fn view(&self, stats: Option<Stats>) -> AppView {
        let spec = self.spec.read().unwrap().clone();
        let rt = self.rt.lock().unwrap();
        let state = rt.state.unwrap_or(AppState::Stopped);
        drop(rt);
        let env_file_status = self.env_file_status(&spec, state);
        let rt = self.rt.lock().unwrap();
        // A user stop ends the run; its errors are no longer news.
        let issue = (spec.watch_errors && state != AppState::Stopped)
            .then(|| self.logs.issue.lock().unwrap().current(now_ms()))
            .flatten();
        AppView {
            spec,
            state,
            pid: rt.pid,
            exit_code: rt.exit_code,
            started_at: rt.started_at,
            restarts: rt.restarts,
            next_restart_at: rt.next_restart_at,
            stats,
            task: self.task.lock().unwrap().run.clone(),
            issue,
            env_file_status,
        }
    }
}

/// What started a run.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Trigger {
    User,
    Boot,
    Restart,
}

impl Supervisor {
    pub fn load(settings: Arc<Settings>) -> Result<Arc<Self>> {
        std::fs::create_dir_all(&settings.data_dir)?;
        let db = Db::open(&settings.data_dir.join("keeprun.db"))?;
        migrate_json(&settings, &db)?;
        let ncpu = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1) as f32;
        let sup = Arc::new(Supervisor {
            settings,
            db,
            apps: RwLock::new(BTreeMap::new()),
            ncpu,
        });
        {
            let mut apps = sup.apps.write().unwrap();
            for stored in sup.db.load_apps()? {
                apps.insert(stored.spec.id.clone(), sup.managed(stored.spec));
            }
        }
        Ok(sup)
    }

    /// Start apps the user left running, when they allow start with the agent.
    pub fn autostart(self: &Arc<Self>) -> Result<()> {
        for stored in self.db.load_apps()? {
            if stored.desired_running && stored.spec.autostart && !stored.spec.disabled {
                if let Ok(m) = self.get(&stored.spec.id) {
                    if let Some(generation) = Self::begin(&m, None, false) {
                        tracing::info!(app = %stored.spec.id, "autostart");
                        let _ = self.spawn_run(&m, generation, Trigger::Boot);
                    }
                }
            }
        }
        Ok(())
    }

    fn managed(&self, spec: AppSpec) -> Arc<Managed> {
        let logs = LogHub::new(self.settings.logs_dir().join(&spec.id));
        Arc::new(Managed {
            spec: RwLock::new(spec),
            rt: Mutex::new(Runtime::default()),
            logs,
            task: Mutex::new(TaskSlot::default()),
            env_cache: Mutex::default(),
        })
    }

    fn get(&self, id: &str) -> Result<Arc<Managed>, SupError> {
        self.apps.read().unwrap().get(id).cloned().ok_or_else(|| {
            SupError::NotFound(format!(
                "No app with id \"{id}\". Run `keeprun list` to see app ids."
            ))
        })
    }

    fn event(&self, id: &str, kind: EventKind, message: impl Into<String>) {
        if let Err(e) = self.db.add_event(id, kind, &message.into()) {
            tracing::warn!("cannot write event: {e:#}");
        }
    }

    pub fn list(&self) -> Vec<AppView> {
        let apps: Vec<Arc<Managed>> = self.apps.read().unwrap().values().cloned().collect();
        let ports = if apps.iter().any(|m| m.state() == AppState::Running) {
            job::listening_ports()
        } else {
            vec![]
        };
        apps.iter().map(|m| m.view(self.stats(m, &ports))).collect()
    }

    pub fn view(&self, id: &str) -> Result<AppView, SupError> {
        let m = self.get(id)?;
        let ports = if m.state() == AppState::Running {
            job::listening_ports()
        } else {
            vec![]
        };
        Ok(m.view(self.stats(&m, &ports)))
    }

    fn stats(&self, m: &Managed, ports: &[(u32, u16)]) -> Option<Stats> {
        let mut rt = m.rt.lock().unwrap();
        let job = rt.job.clone()?;
        let pids = job.process_ids();
        let cpu = job.cpu_time();
        let now = Instant::now();
        if let Some((t, prev)) = rt.cpu_sample {
            let elapsed = now.duration_since(t).as_secs_f32();
            // Sample at most every 500 ms so frequent polls do not produce noise.
            if elapsed >= 0.5 {
                let used = (cpu.saturating_sub(prev)) as f32 / 10_000_000.0;
                rt.cpu_percent = (used / elapsed / self.ncpu * 100.0).clamp(0.0, 100.0);
                rt.cpu_sample = Some((now, cpu));
            }
        } else {
            rt.cpu_sample = Some((now, cpu));
        }
        let cpu_percent = rt.cpu_percent;
        drop(rt);
        let set: HashSet<u32> = pids.iter().copied().collect();
        let mut app_ports: Vec<u16> = ports
            .iter()
            .filter(|(p, _)| set.contains(p))
            .map(|(_, port)| *port)
            .collect();
        app_ports.sort();
        app_ports.dedup();
        Some(Stats {
            cpu_percent,
            memory_bytes: pids.iter().map(|&p| job::memory_of(p)).sum(),
            processes: pids.len() as u32,
            ports: app_ports,
        })
    }

    fn validate(name: &str, cwd: &std::path::Path, command: &str) -> Result<(), SupError> {
        if slugify(name).is_empty() {
            return Err(SupError::BadRequest(
                "App name needs at least one letter or digit.".into(),
            ));
        }
        if command.trim().is_empty() {
            return Err(SupError::BadRequest(
                "Command is empty. Enter the command that starts the app.".into(),
            ));
        }
        if !cwd.is_dir() {
            return Err(SupError::BadRequest(format!(
                "Folder {} does not exist. Pick an existing folder.",
                cwd.display()
            )));
        }
        Ok(())
    }

    pub fn add(&self, new: NewApp) -> Result<AppView, SupError> {
        Self::validate(&new.name, &new.cwd, &new.command)?;
        let tasks = clean_tasks(new.tasks)?;
        let id = slugify(&new.name);
        let spec = AppSpec {
            id: id.clone(),
            name: new.name.trim().into(),
            cwd: new.cwd,
            command: new.command.trim().into(),
            secrets: new
                .secrets
                .into_iter()
                .filter(|k| new.env.contains_key(k))
                .collect(),
            env: new.env,
            tags: clean_tags(new.tags),
            autostart: new.autostart,
            restart: new.restart,
            max_restarts: new.max_restarts,
            tasks,
            disabled: false,
            watch_errors: new.watch_errors,
            env_file: clean_env_file(new.env_file),
        };
        {
            let mut apps = self.apps.write().unwrap();
            Self::port_taken(&apps, &id, &Self::spec_env(&spec))?;
            if apps.contains_key(&id) {
                return Err(SupError::Conflict(format!(
                    "An app with id \"{id}\" already exists. Choose another name."
                )));
            }
            self.db.insert_app(&spec).map_err(|e| {
                if format!("{e:#}").contains("UNIQUE") {
                    SupError::Conflict(format!(
                        "An app with id \"{id}\" exists in the database but could not be loaded. Check agent.log."
                    ))
                } else {
                    SupError::Internal(e)
                }
            })?;
            apps.insert(id.clone(), self.managed(spec));
        }
        self.event(&id, EventKind::Added, "App added");
        tracing::info!(app = %id, "app added");
        self.view(&id)
    }

    pub fn update(&self, id: &str, u: UpdateApp) -> Result<AppView, SupError> {
        let m = self.get(id)?;
        let mut spec = m.spec.read().unwrap().clone();
        let before = spec.clone();
        if let Some(v) = u.name {
            spec.name = v.trim().into();
        }
        if let Some(v) = u.cwd {
            spec.cwd = v;
        }
        if let Some(v) = u.command {
            spec.command = v.trim().into();
        }
        if let Some(v) = u.env {
            spec.env = v;
        }
        if let Some(v) = u.secrets {
            spec.secrets = v;
        }
        spec.secrets.retain(|k| spec.env.contains_key(k));
        if let Some(v) = u.tags {
            spec.tags = clean_tags(v);
        }
        if let Some(v) = u.autostart {
            spec.autostart = v;
        }
        if let Some(v) = u.restart {
            spec.restart = v;
        }
        if let Some(v) = u.max_restarts {
            spec.max_restarts = v;
        }
        if let Some(v) = u.tasks {
            spec.tasks = clean_tasks(v)?;
        }
        if let Some(v) = u.disabled {
            if v && !before.disabled && m.state().is_active() {
                return Err(SupError::Conflict(format!(
                    "App \"{id}\" is running. Stop it before disabling it."
                )));
            }
            spec.disabled = v;
        }
        if let Some(v) = u.watch_errors {
            spec.watch_errors = v;
            if !v {
                m.logs.clear_issue();
            }
        }
        if let Some(v) = u.env_file {
            spec.env_file = clean_env_file(Some(v));
        }
        Self::validate(&spec.name, &spec.cwd, &spec.command)?;
        let env = Self::spec_env(&spec);
        if job::port_of(&env) != job::port_of(&Self::spec_env(&before)) {
            Self::port_taken(&self.apps.read().unwrap(), id, &env)?;
        }
        if spec == before {
            return self.view(id);
        }
        self.db.update_app(&spec)?;
        let changed = describe_changes(&before, &spec);
        *m.spec.write().unwrap() = spec;
        let running = m.state().is_active();
        let note = if running && needs_restart(&before, &m.spec.read().unwrap()) {
            " Restart to apply."
        } else {
            ""
        };
        self.event(
            id,
            EventKind::ConfigChanged,
            format!("Changed {changed}.{note}"),
        );
        self.view(id)
    }

    pub fn remove(&self, id: &str) -> Result<(), SupError> {
        let m = self.get(id)?;
        if m.state().is_active() {
            return Err(SupError::Conflict(format!(
                "App \"{id}\" is running. Stop it before removing it."
            )));
        }
        if m.task.lock().unwrap().job.is_some() {
            return Err(SupError::Conflict(format!(
                "A task is running for \"{id}\". Wait for it or stop it before removing the app."
            )));
        }
        self.db.delete_app(id)?;
        self.apps.write().unwrap().remove(id);
        tracing::info!(app = %id, "app removed");
        Ok(())
    }

    pub fn events(&self, id: Option<&str>, limit: u32) -> Result<Vec<Event>, SupError> {
        if let Some(id) = id {
            self.get(id)?;
        }
        Ok(self.db.events(id, limit.clamp(1, 1000))?)
    }

    /// Errors the app printed in this run, and a prompt a coding agent can work from.
    pub fn issue(&self, id: &str) -> Result<IssueReport, SupError> {
        let view = self.view(id)?;
        let events = if view.issue.is_some() {
            self.events(Some(id), 10)?
        } else {
            vec![]
        };
        Ok(IssueReport {
            prompt: issue::agent_prompt(&view, &events),
            issue: view.issue,
        })
    }

    /// Forget the current issue. A new error starts a new one.
    pub fn clear_issue(&self, id: &str) -> Result<AppView, SupError> {
        self.get(id)?.logs.clear_issue();
        self.view(id)
    }

    pub fn recent_logs(&self, id: &str, tail: usize) -> Result<Vec<LogLine>, SupError> {
        Ok(self.get(id)?.logs.recent(tail))
    }

    pub fn subscribe_logs(
        &self,
        id: &str,
    ) -> Result<(Vec<LogLine>, broadcast::Receiver<LogLine>), SupError> {
        let m = self.get(id)?;
        // Subscribe first so no line falls between the snapshot and the live stream.
        let rx = m.logs.subscribe();
        Ok((m.logs.recent(LOG_BUFFER_LINES), rx))
    }

    /// User start. Clears a Failed state and the restart counter.
    pub fn start(self: &Arc<Self>, id: &str) -> Result<AppView, SupError> {
        let m = self.get(id)?;
        if m.spec.read().unwrap().disabled {
            return Err(SupError::Conflict(format!(
                "App \"{id}\" is disabled. Enable it before starting it."
            )));
        }
        let Some(generation) = Self::begin(&m, None, true) else {
            let s = m.state().to_string().to_lowercase();
            return Err(SupError::Conflict(format!("App \"{id}\" is already {s}.")));
        };
        self.spawn_run(&m, generation, Trigger::User)?;
        // A stop may have won the race while the process was being created.
        if m.state().is_active() {
            self.db.set_desired_running(id, true)?;
        }
        self.view(id)
    }

    /// Two apps set to the same port would fight over it. Refuse when saving, not at 3 AM.
    fn port_taken(
        apps: &BTreeMap<String, Arc<Managed>>,
        id: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<(), SupError> {
        let Some(port) = job::port_of(env) else {
            return Ok(());
        };
        for (other, m) in apps.iter().filter(|(k, _)| k.as_str() != id) {
            let spec = m.spec.read().unwrap().clone();
            if job::port_of(&m.effective_env(&spec).0) == Some(port) {
                return Err(SupError::Conflict(format!(
                    "Port {port} is already set for app \"{other}\". Choose another port."
                )));
            }
        }
        Ok(())
    }

    /// Env of an app that is not managed yet, uncached: the env file under the app variables.
    fn spec_env(spec: &AppSpec) -> EnvVars {
        let mut env = spec
            .env_file
            .as_deref()
            .and_then(|f| read_env_file(&spec.cwd, f).ok())
            .unwrap_or_default();
        env.extend(spec.env.clone());
        env
    }

    /// Port given in the app env as PORT, if another process already listens on it.
    fn port_conflict(env: &EnvVars) -> Option<String> {
        let port = job::port_of(env)?;
        let (pid, _) = job::listening_ports()
            .into_iter()
            .find(|(_, p)| *p == port)?;
        let name = job::process_name(pid).unwrap_or_else(|| "another process".into());
        Some(format!(
            "Port {port} is in use by {name} (PID {pid}). Stop it or choose another port."
        ))
    }

    /// Claim a new run in one critical section, so two starts, or a start and a stop, cannot
    /// interleave. `expect` is the generation a restart timer was armed with: the run is only
    /// claimed when the app is still waiting on that timer. Without it, the app must be idle.
    fn begin(m: &Managed, expect: Option<u64>, reset_restarts: bool) -> Option<u64> {
        let mut rt = m.rt.lock().unwrap();
        match expect {
            Some(g) if rt.generation != g || rt.state != Some(AppState::Restarting) => return None,
            None if rt.state.is_some_and(|s| s.is_active()) => return None,
            _ => {}
        }
        rt.generation += 1;
        rt.state = Some(AppState::Starting);
        rt.exit_code = None;
        rt.next_restart_at = None;
        if reset_restarts {
            rt.restarts = 0;
        }
        Some(rt.generation)
    }

    fn spawn_run(
        self: &Arc<Self>,
        m: &Arc<Managed>,
        generation: u64,
        trigger: Trigger,
    ) -> Result<(), SupError> {
        let spec = m.spec.read().unwrap().clone();
        let id = spec.id.clone();

        let (env, file) = m.effective_env(&spec);
        // A missing or broken env file is reported but does not stop the app from starting:
        // keeping it up comes first, and the status shows the error.
        if let Some(Err(e)) = &file {
            m.logs
                .push(Stream::System, format!("Env file not loaded: {e}"));
        }
        m.rt.lock().unwrap().env_file_loaded = file.and_then(Result::ok);
        let started = Self::port_conflict(&env).map_or_else(
            || job::spawn_in_job(&spec.cwd, &spec.command, &env),
            |msg| Err(anyhow!(msg)),
        );
        let (mut child, job) = match started {
            Ok(v) => v,
            Err(e) => {
                let msg = format!("{e:#}");
                m.logs.push(Stream::System, format!("Start failed: {msg}"));
                self.event(&id, EventKind::StartFailed, msg.clone());
                if trigger == Trigger::User {
                    let mut rt = m.rt.lock().unwrap();
                    if rt.generation == generation && rt.state == Some(AppState::Starting) {
                        rt.state = Some(AppState::Failed);
                    } else if rt.generation == generation && rt.state == Some(AppState::Stopping) {
                        rt.state = Some(AppState::Stopped);
                    }
                    return Err(SupError::BadRequest(msg));
                }
                // Boot and restarts keep trying, e.g. a network drive that mounts late.
                self.after_exit(m, generation, None, Duration::ZERO);
                return Ok(());
            }
        };

        // A new run starts clean. Errors from the previous one may already be fixed.
        m.logs.clear_issue();
        m.logs.push(
            Stream::System,
            format!("Started: {} (in {})", spec.command, spec.cwd.display()),
        );
        let pid = child.id();
        let restarts = {
            let mut rt = m.rt.lock().unwrap();
            if rt.generation != generation || rt.state != Some(AppState::Starting) {
                // A stop arrived while the process was being created. End it right away.
                if rt.generation == generation && rt.state == Some(AppState::Stopping) {
                    rt.state = Some(AppState::Stopped);
                }
                drop(rt);
                let _ = child.start_kill();
                job.terminate(1);
                m.logs
                    .push(Stream::System, "Stopped before start finished".into());
                return Ok(());
            }
            rt.state = Some(AppState::Running);
            rt.pid = pid;
            rt.started_at = Some(now_ms());
            rt.started_instant = Some(Instant::now());
            rt.job = Some(Arc::new(job));
            rt.cpu_sample = None;
            rt.cpu_percent = 0.0;
            rt.restarts
        };
        let pid_s = pid.map(|p| format!(" (PID {p})")).unwrap_or_default();
        match trigger {
            Trigger::User => self.event(&id, EventKind::Started, format!("Started{pid_s}")),
            Trigger::Boot => self.event(
                &id,
                EventKind::Started,
                format!("Started with the agent{pid_s}"),
            ),
            Trigger::Restart => self.event(
                &id,
                EventKind::Restarted,
                format!("Restarted after exit, attempt {restarts}{pid_s}"),
            ),
        }
        tracing::info!(app = %id, pid = ?pid, "started");

        if let Some(out) = child.stdout.take() {
            tokio::spawn(pump(out, m.clone(), Stream::Stdout, false));
        }
        if let Some(err) = child.stderr.take() {
            tokio::spawn(pump(err, m.clone(), Stream::Stderr, false));
        }
        let sup = self.clone();
        let m2 = m.clone();
        tokio::spawn(async move {
            let status = child.wait().await;
            let code = status.as_ref().ok().and_then(|s| s.code());
            let ran = m2
                .rt
                .lock()
                .unwrap()
                .started_instant
                .map(|t| t.elapsed())
                .unwrap_or_default();
            sup.after_exit(&m2, generation, code, ran);
        });
        Ok(())
    }

    /// Decide what happens after a run ends: Stopped, Exited, Restarting or Failed.
    fn after_exit(
        self: &Arc<Self>,
        m: &Arc<Managed>,
        generation: u64,
        code: Option<i32>,
        ran: Duration,
    ) {
        let spec = m.spec.read().unwrap().clone();
        let mut rt = m.rt.lock().unwrap();
        if rt.generation != generation {
            return;
        }
        // Dropping the job closes it and kills any process left in the tree.
        rt.job = None;
        rt.pid = None;
        rt.exit_code = code;
        let code_s = code
            .map(|c| format!("code {c}"))
            .unwrap_or_else(|| "no exit code".into());

        if rt.state == Some(AppState::Stopping) {
            rt.state = Some(AppState::Stopped);
            drop(rt);
            m.logs.push(Stream::System, format!("Stopped ({code_s})"));
            self.event(&spec.id, EventKind::Stopped, "Stopped");
            return;
        }

        if ran >= STABLE_RUN {
            rt.restarts = 0;
        }
        let failed = code != Some(0);
        let restart = match spec.restart {
            RestartPolicy::Never => false,
            RestartPolicy::OnFailure => failed,
            RestartPolicy::Always => true,
        };
        if !restart {
            rt.state = Some(AppState::Exited);
            drop(rt);
            m.logs.push(Stream::System, format!("Exited with {code_s}"));
            let kind = if failed {
                EventKind::Crashed
            } else {
                EventKind::Exited
            };
            self.event(&spec.id, kind, format!("Exited with {code_s}"));
            return;
        }
        if spec.max_restarts > 0 && rt.restarts >= spec.max_restarts {
            rt.state = Some(AppState::Failed);
            drop(rt);
            let msg = format!("Exited with {code_s}. Gave up after {} restarts in a row. Check the logs, then Start.", spec.max_restarts);
            m.logs.push(Stream::System, msg.clone());
            self.event(&spec.id, EventKind::Failed, msg);
            return;
        }
        rt.restarts += 1;
        let n = rt.restarts;
        let delay = backoff(n);
        rt.state = Some(AppState::Restarting);
        rt.next_restart_at = Some(now_ms() + delay.as_millis() as u64);
        drop(rt);
        let of = if spec.max_restarts > 0 {
            format!(" of {}", spec.max_restarts)
        } else {
            String::new()
        };
        let msg = format!(
            "Exited with {code_s}. Restarting in {}s (attempt {n}{of}).",
            delay.as_secs()
        );
        m.logs.push(Stream::System, msg.clone());
        self.event(
            &spec.id,
            if failed {
                EventKind::Crashed
            } else {
                EventKind::Exited
            },
            msg,
        );

        let sup = self.clone();
        let m = m.clone();
        tokio::spawn(async move {
            tokio::time::sleep(delay).await;
            if let Some(next) = Self::begin(&m, Some(generation), false) {
                let _ = sup.spawn_run(&m, next, Trigger::Restart);
            }
        });
    }

    /// User stop. The app stays stopped across agent restarts.
    pub async fn stop(self: &Arc<Self>, id: &str) -> Result<AppView, SupError> {
        self.db.set_desired_running(id, false).ok();
        self.stop_inner(id).await?;
        self.view(id)
    }

    async fn stop_inner(&self, id: &str) -> Result<(), SupError> {
        let m = self.get(id)?;
        let (pid, job, generation) = {
            let mut rt = m.rt.lock().unwrap();
            match rt.state {
                Some(AppState::Running | AppState::Starting) => {}
                Some(AppState::Stopping) => {
                    return Err(SupError::Conflict(format!(
                        "App \"{id}\" is already stopping."
                    )))
                }
                Some(AppState::Restarting) => {
                    // Cancel the pending restart.
                    rt.generation += 1;
                    rt.state = Some(AppState::Stopped);
                    rt.next_restart_at = None;
                    drop(rt);
                    self.event(
                        id,
                        EventKind::Stopped,
                        "Stopped. Pending restart cancelled.",
                    );
                    return Ok(());
                }
                Some(AppState::Failed | AppState::Exited) => {
                    rt.state = Some(AppState::Stopped);
                    return Ok(());
                }
                _ => return Ok(()),
            }
            rt.state = Some(AppState::Stopping);
            (rt.pid, rt.job.clone(), rt.generation)
        };
        let timeout = Duration::from_millis(self.settings.stop_timeout_ms);
        m.logs.push(Stream::System, "Stopping (Ctrl+Break)".into());

        let sent = pid.map(job::send_ctrl_break).unwrap_or(false);
        if !sent || !wait_until_done(&m, generation, timeout).await {
            m.logs.push(
                Stream::System,
                "Did not stop in time. Ending the process tree.".into(),
            );
            if let Some(job) = &job {
                job.terminate(1);
            }
            if !wait_until_done(&m, generation, Duration::from_secs(5)).await {
                return Err(SupError::Internal(anyhow!(
                    "App \"{id}\" did not stop after its process tree was ended."
                )));
            }
        }
        Ok(())
    }

    /// Run a saved task or a one-off command in the app folder with the app environment.
    /// Returns once the process has started. Output goes to the app log, marked as task output.
    pub fn run_task(self: &Arc<Self>, id: &str, req: RunTask) -> Result<AppView, SupError> {
        let m = self.get(id)?;
        let spec = m.spec.read().unwrap().clone();
        let (name, command) = match (req.name, req.command) {
            (Some(n), None) => {
                let t = spec.tasks.iter().find(|t| t.name == n).ok_or_else(|| {
                    SupError::NotFound(format!(
                        "App \"{id}\" has no task named \"{n}\". Run `keeprun tasks {id}` to see them."
                    ))
                })?;
                (Some(n), t.command.clone())
            }
            (None, Some(c)) if !c.trim().is_empty() => (None, c.trim().to_string()),
            _ => {
                return Err(SupError::BadRequest(
                    "Give either a task name or a command.".into(),
                ))
            }
        };
        let label = name.clone().unwrap_or_else(|| command.clone());

        let seq = {
            let mut slot = m.task.lock().unwrap();
            if let Some(r) = slot.run.as_ref().filter(|r| r.running) {
                return Err(SupError::Conflict(format!(
                    "A task is already running for \"{id}\": {}. Wait for it or stop it.",
                    r.name.as_deref().unwrap_or(&r.command)
                )));
            }
            slot.seq += 1;
            slot.stop_requested = false;
            slot.run = Some(TaskRun {
                name: name.clone(),
                command: command.clone(),
                running: true,
                started_at: now_ms(),
                finished_at: None,
                exit_code: None,
            });
            slot.seq
        };

        let env = m.effective_env(&spec).0;
        let (mut child, job) = match job::spawn_in_job(&spec.cwd, &command, &env) {
            Ok(v) => v,
            Err(e) => {
                let msg = format!("{e:#}");
                if let Some(r) = m.task.lock().unwrap().run.as_mut() {
                    r.running = false;
                    r.finished_at = Some(now_ms());
                }
                m.logs
                    .push_line(Stream::System, format!("Task did not start: {msg}"), true);
                self.event(
                    id,
                    EventKind::TaskFailed,
                    format!("Task {label} did not start: {msg}"),
                );
                return Err(SupError::BadRequest(msg));
            }
        };
        m.task.lock().unwrap().job = Some(Arc::new(job));
        m.logs
            .push_line(Stream::System, format!("Task started: {command}"), true);
        self.event(id, EventKind::TaskStarted, format!("Task {label} started"));
        tracing::info!(app = %id, task = %label, "task started");

        let mut pumps = vec![];
        if let Some(out) = child.stdout.take() {
            pumps.push(tokio::spawn(pump(out, m.clone(), Stream::Stdout, true)));
        }
        if let Some(err) = child.stderr.take() {
            pumps.push(tokio::spawn(pump(err, m.clone(), Stream::Stderr, true)));
        }
        let sup = self.clone();
        let m2 = m.clone();
        let id2 = id.to_string();
        tokio::spawn(async move {
            let code = child.wait().await.ok().and_then(|s| s.code());
            // Ending the job closes pipes held by leftover child processes, so the output
            // is complete before the finish line. Clients use that line as the end marker.
            let job = m2.task.lock().unwrap().job.clone();
            if let Some(job) = job {
                job.terminate(1);
            }
            let flush = futures_util::future::join_all(pumps);
            let _ = tokio::time::timeout(Duration::from_secs(2), flush).await;
            let stopped = {
                let mut slot = m2.task.lock().unwrap();
                if slot.seq != seq {
                    return;
                }
                // Dropping the job ends any process the task left behind.
                slot.job = None;
                if let Some(r) = slot.run.as_mut() {
                    r.running = false;
                    r.finished_at = Some(now_ms());
                    r.exit_code = code;
                }
                slot.stop_requested
            };
            let code_s = code.map_or_else(|| "no exit code".into(), |c| format!("exit code {c}"));
            let (kind, msg, line) = if stopped {
                (
                    EventKind::TaskFailed,
                    format!("Task {label} stopped"),
                    "Task stopped".to_string(),
                )
            } else if code == Some(0) {
                (
                    EventKind::TaskFinished,
                    format!("Task {label} finished"),
                    "Task finished with exit code 0".to_string(),
                )
            } else {
                (
                    EventKind::TaskFailed,
                    format!("Task {label} failed with {code_s}"),
                    format!("Task failed with {code_s}"),
                )
            };
            m2.logs.push_line(Stream::System, line, true);
            sup.event(&id2, kind, msg);
        });
        self.view(id)
    }

    /// End the running task and its process tree.
    pub fn stop_task(&self, id: &str) -> Result<AppView, SupError> {
        let m = self.get(id)?;
        {
            let mut slot = m.task.lock().unwrap();
            let Some(job) = slot.job.clone() else {
                return Err(SupError::Conflict(format!(
                    "No task is running for \"{id}\"."
                )));
            };
            slot.stop_requested = true;
            job.terminate(1);
        }
        self.view(id)
    }

    pub async fn restart(self: &Arc<Self>, id: &str) -> Result<AppView, SupError> {
        self.stop_inner(id).await?;
        self.start(id)
    }

    /// Stop every running app without changing desired state. Used on agent shutdown.
    pub async fn stop_all(&self) {
        for m in self.apps.read().unwrap().values() {
            if let Some(job) = m.task.lock().unwrap().job.clone() {
                job.terminate(1);
            }
        }
        let ids: Vec<String> = self
            .apps
            .read()
            .unwrap()
            .iter()
            .filter(|(_, m)| m.state().is_active())
            .map(|(k, _)| k.clone())
            .collect();
        let futs = ids.iter().map(|id| self.stop_inner(id));
        for r in futures_util::future::join_all(futs).await {
            if let Err(e) = r {
                tracing::warn!("stop on shutdown: {e}");
            }
        }
    }
}

fn clean_tags(tags: Vec<String>) -> Vec<String> {
    let mut t: Vec<String> = tags
        .into_iter()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .collect();
    t.sort();
    t.dedup();
    t
}

/// Trim names and commands, drop empty rows, and reject duplicate names.
fn clean_tasks(tasks: Vec<TaskSpec>) -> Result<Vec<TaskSpec>, SupError> {
    let mut out: Vec<TaskSpec> = vec![];
    for t in tasks {
        let name = t.name.trim().to_string();
        let command = t.command.trim().to_string();
        if name.is_empty() && command.is_empty() {
            continue;
        }
        if name.is_empty() || command.is_empty() {
            return Err(SupError::BadRequest(
                "Each task needs a name and a command.".into(),
            ));
        }
        if out.iter().any(|o| o.name == name) {
            return Err(SupError::BadRequest(format!(
                "Two tasks are named \"{name}\". Give each task its own name."
            )));
        }
        out.push(TaskSpec { name, command });
    }
    Ok(out)
}

fn needs_restart(a: &AppSpec, b: &AppSpec) -> bool {
    a.cwd != b.cwd || a.command != b.command || a.env != b.env || a.env_file != b.env_file
}

/// Trim an env file name. Empty means no file.
fn clean_env_file(v: Option<String>) -> Option<String> {
    v.map(|f| f.trim().to_string()).filter(|f| !f.is_empty())
}

fn describe_changes(a: &AppSpec, b: &AppSpec) -> String {
    let mut parts = vec![];
    if a.name != b.name {
        parts.push("name".to_string());
    }
    if a.cwd != b.cwd {
        parts.push("folder".into());
    }
    if a.command != b.command {
        parts.push("command".into());
    }
    if a.env_file != b.env_file {
        parts.push(match &b.env_file {
            Some(f) => format!("env file to {f}"),
            None => "env file (unlinked)".into(),
        });
    }
    if a.env != b.env || a.secrets != b.secrets {
        let mut keys: Vec<&String> = a
            .env
            .keys()
            .chain(b.env.keys())
            .filter(|k| a.env.get(*k) != b.env.get(*k))
            .collect();
        keys.sort();
        keys.dedup();
        parts.push(if keys.is_empty() {
            "environment".into()
        } else {
            format!(
                "environment ({})",
                keys.iter()
                    .map(|k| k.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        });
    }
    if a.tags != b.tags {
        parts.push("tags".into());
    }
    if a.autostart != b.autostart {
        parts.push(format!(
            "start with agent to {}",
            if b.autostart { "on" } else { "off" }
        ));
    }
    if a.restart != b.restart {
        parts.push(format!("restart policy to {}", b.restart.as_str()));
    }
    if a.max_restarts != b.max_restarts {
        parts.push(format!("max restarts to {}", b.max_restarts));
    }
    if a.tasks != b.tasks {
        parts.push("tasks".into());
    }
    if a.watch_errors != b.watch_errors {
        parts.push(format!(
            "watch for errors to {}",
            if b.watch_errors { "on" } else { "off" }
        ));
    }
    if a.disabled != b.disabled {
        parts.push(
            if b.disabled {
                "state to disabled"
            } else {
                "state to enabled"
            }
            .into(),
        );
    }
    parts.join(", ")
}

/// Import apps.json written by the M0 agent, once.
fn migrate_json(settings: &Settings, db: &Db) -> Result<()> {
    let path = settings.apps_path();
    if !path.is_file() || !db.is_empty()? {
        return Ok(());
    }
    let text = std::fs::read_to_string(&path)?;
    let specs: Vec<AppSpec> = serde_json::from_str(&text)
        .with_context(|| format!("Invalid app list in {}", path.display()))?;
    for s in &specs {
        db.insert_app(s)?;
    }
    std::fs::rename(&path, path.with_extension("json.migrated"))?;
    tracing::info!(count = specs.len(), "imported apps.json into the database");
    Ok(())
}

async fn wait_until_done(m: &Managed, generation: u64, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        {
            let rt = m.rt.lock().unwrap();
            if rt.generation != generation || rt.state != Some(AppState::Stopping) {
                return true;
            }
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn pump<R: AsyncRead + Unpin>(reader: R, m: Arc<Managed>, stream: Stream, task: bool) {
    use tokio::io::AsyncReadExt;
    // Cap line length so output without newlines cannot grow memory without bound.
    const MAX_LINE: u64 = 64 * 1024;
    let mut reader = BufReader::new(reader);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match (&mut reader)
            .take(MAX_LINE)
            .read_until(b'\n', &mut buf)
            .await
        {
            Ok(0) | Err(_) => break,
            Ok(_) => {
                let text = String::from_utf8_lossy(&buf);
                m.logs.push_line(
                    stream,
                    text.trim_end_matches(['\r', '\n']).to_string(),
                    task,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use keeprun_core::Env;

    /// Supervisor on a fresh temp data folder.
    fn sup(name: &str) -> Arc<Supervisor> {
        let dir = std::env::temp_dir().join(format!("keeprun-sup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut s = Settings::defaults(Env::Dev);
        s.data_dir = dir;
        s.stop_timeout_ms = 1500;
        Supervisor::load(Arc::new(s)).unwrap()
    }

    fn app(s: &Supervisor, name: &str, command: &str, restart: RestartPolicy, max: u32) -> String {
        s.add(NewApp {
            name: name.into(),
            cwd: std::env::temp_dir(),
            command: command.into(),
            env: Default::default(),
            secrets: Default::default(),
            tags: vec![],
            autostart: true,
            restart,
            max_restarts: max,
            tasks: vec![TaskSpec {
                name: "hello".into(),
                command: "echo task-output".into(),
            }],
            watch_errors: true,
            env_file: None,
        })
        .unwrap()
        .spec
        .id
    }

    async fn wait_for(s: &Supervisor, id: &str, want: AppState, secs: u64) -> AppState {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let st = s.view(id).unwrap().state;
            if st == want || Instant::now() > deadline {
                return st;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn wait_task(s: &Supervisor, id: &str, secs: u64) -> TaskRun {
        let deadline = Instant::now() + Duration::from_secs(secs);
        loop {
            let t = s.view(id).unwrap().task.unwrap();
            if !t.running || Instant::now() > deadline {
                return t;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    #[tokio::test]
    async fn env_file_is_read_on_start_and_changes_are_reported() {
        let s = sup("envfile");
        let dir = std::env::temp_dir().join(format!("keeprun-envfile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(".env.local"), "GREETING=hi\nOTHER=1\n").unwrap();
        let id = app(
            &s,
            "envfile",
            "echo greet-%GREETING%-%OTHER% & ping -n 60 127.0.0.1 >nul",
            RestartPolicy::Never,
            5,
        );
        let v = s
            .update(
                &id,
                UpdateApp {
                    cwd: Some(dir.clone()),
                    env_file: Some(".env.local".into()),
                    env: Some(BTreeMap::from([("OTHER".into(), "app".into())])),
                    ..Default::default()
                },
            )
            .unwrap();
        let status = v.env_file_status.unwrap();
        assert_eq!(status.keys, ["GREETING", "OTHER"]);
        assert!(status.changed.is_empty() && status.error.is_none());

        s.start(&id).unwrap();
        wait_for(&s, &id, AppState::Running, 10).await;
        let deadline = Instant::now() + Duration::from_secs(10);
        // App variables override the file.
        while !s
            .recent_logs(&id, 50)
            .unwrap()
            .iter()
            .any(|l| l.text.contains("greet-hi-app"))
        {
            assert!(
                Instant::now() < deadline,
                "env file values not in the app output"
            );
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        std::fs::write(dir.join(".env.local"), "GREETING=hello\nOTHER=1\nNEW=x\n").unwrap();
        assert_eq!(
            s.view(&id).unwrap().env_file_status.unwrap().changed,
            ["GREETING", "NEW"]
        );

        std::fs::remove_file(dir.join(".env.local")).unwrap();
        let status = s.view(&id).unwrap().env_file_status.unwrap();
        assert!(status.error.unwrap().contains("does not exist"));
        assert_eq!(status.changed, ["GREETING", "OTHER"]);

        // Not running: nothing to restart for.
        s.stop(&id).await.unwrap();
        wait_for(&s, &id, AppState::Stopped, 10).await;
        assert!(s
            .view(&id)
            .unwrap()
            .env_file_status
            .unwrap()
            .changed
            .is_empty());
    }

    #[tokio::test]
    async fn disabled_apps_do_not_start() {
        let s = sup("disabled");
        let id = app(
            &s,
            "sleeper",
            "ping -n 60 127.0.0.1 >nul",
            RestartPolicy::Never,
            5,
        );
        let disable = || UpdateApp {
            disabled: Some(true),
            ..Default::default()
        };

        // A running app must be stopped first.
        s.start(&id).unwrap();
        assert!(matches!(
            s.update(&id, disable()),
            Err(SupError::Conflict(_))
        ));
        s.stop(&id).await.unwrap();
        wait_for(&s, &id, AppState::Stopped, 10).await;

        assert!(s.update(&id, disable()).unwrap().spec.disabled);
        assert!(matches!(s.start(&id), Err(SupError::Conflict(_))));

        let enabled = s
            .update(
                &id,
                UpdateApp {
                    disabled: Some(false),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(!enabled.spec.disabled);
        assert_eq!(enabled.state, AppState::Stopped);
        s.start(&id).unwrap();
        s.stop(&id).await.unwrap();
    }

    #[tokio::test]
    async fn tasks_run_once_and_are_marked_in_logs() {
        let s = sup("tasks");
        let id = app(
            &s,
            "tasker",
            "ping -n 60 127.0.0.1 >nul",
            RestartPolicy::Never,
            5,
        );
        s.run_task(
            &id,
            RunTask {
                name: Some("hello".into()),
                command: None,
            },
        )
        .unwrap();
        let t = wait_task(&s, &id, 10).await;
        assert_eq!(t.exit_code, Some(0));
        assert_eq!(t.name.as_deref(), Some("hello"));
        // Give the output pump a moment after the exit.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let logs = s.recent_logs(&id, 50).unwrap();
        assert!(logs
            .iter()
            .any(|l| l.task && l.text.contains("task-output")));
        // The app itself was never started by the task.
        assert_eq!(s.view(&id).unwrap().state, AppState::Stopped);

        // A failing one-off command reports its exit code.
        s.run_task(
            &id,
            RunTask {
                name: None,
                command: Some("exit /b 3".into()),
            },
        )
        .unwrap();
        assert_eq!(wait_task(&s, &id, 10).await.exit_code, Some(3));

        // Unknown names and a second concurrent task are refused; stop ends a long task.
        assert!(matches!(
            s.run_task(
                &id,
                RunTask {
                    name: Some("nope".into()),
                    command: None
                }
            ),
            Err(SupError::NotFound(_))
        ));
        s.run_task(
            &id,
            RunTask {
                name: None,
                command: Some("ping -n 60 127.0.0.1 >nul".into()),
            },
        )
        .unwrap();
        assert!(matches!(
            s.run_task(
                &id,
                RunTask {
                    name: Some("hello".into()),
                    command: None
                }
            ),
            Err(SupError::Conflict(_))
        ));
        assert!(matches!(s.remove(&id), Err(SupError::Conflict(_))));
        s.stop_task(&id).unwrap();
        assert!(!wait_task(&s, &id, 5).await.running);
        s.remove(&id).unwrap();
    }

    #[test]
    fn two_apps_cannot_share_a_port() {
        let s = sup("ports");
        let a = app(&s, "first", "x", RestartPolicy::Never, 5);
        let b = app(&s, "second", "x", RestartPolicy::Never, 5);
        let port = |p: &str| UpdateApp {
            env: Some(BTreeMap::from([("PORT".into(), p.into())])),
            ..Default::default()
        };
        s.update(&a, port("4100")).unwrap();
        assert!(matches!(
            s.update(&b, port("4100")),
            Err(SupError::Conflict(_))
        ));
        s.update(&b, port("4101")).unwrap();
        // Saving the same app again with its own port is fine.
        s.update(&a, port("4100")).unwrap();
    }

    #[test]
    fn task_list_is_validated() {
        let t = |n: &str, c: &str| TaskSpec {
            name: n.into(),
            command: c.into(),
        };
        assert_eq!(
            clean_tasks(vec![t(" a ", " x "), t("", "")]).unwrap(),
            vec![t("a", "x")]
        );
        assert!(clean_tasks(vec![t("a", "")]).is_err());
        assert!(clean_tasks(vec![t("a", "x"), t("a", "y")]).is_err());
    }

    #[tokio::test]
    async fn errors_in_output_become_an_issue() {
        let s = sup("issue");
        let id = app(
            &s,
            "broken",
            "echo TypeError: boom& echo TypeError: boom& ping -n 60 127.0.0.1 >nul",
            RestartPolicy::Never,
            0,
        );
        s.start(&id).unwrap();
        let mut issue = None;
        for _ in 0..50 {
            issue = s.view(&id).unwrap().issue;
            if issue.as_ref().is_some_and(|i| i.count == 2) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        let issue = issue.expect("no issue from error output");
        assert_eq!(issue.count, 2);
        assert_eq!(issue.summary, "TypeError: boom");
        let report = s.issue(&id).unwrap();
        let prompt = report.prompt.unwrap();
        assert!(prompt.contains("TypeError: boom"));
        assert!(prompt.contains(&format!("keeprun logs {id}")));

        // Task output does not count.
        s.clear_issue(&id).unwrap();
        s.run_task(
            &id,
            RunTask {
                name: None,
                command: Some("echo TypeError: from task".into()),
            },
        )
        .unwrap();
        tokio::time::sleep(Duration::from_millis(800)).await;
        assert!(s.view(&id).unwrap().issue.is_none());

        // Muting hides it.
        s.app_logs_feed_for_test(&id, "ReferenceError: x");
        assert!(s.view(&id).unwrap().issue.is_some());
        s.update(
            &id,
            UpdateApp {
                watch_errors: Some(false),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(s.view(&id).unwrap().issue.is_none());
        s.stop(&id).await.unwrap();
    }

    #[tokio::test]
    async fn start_stop_and_double_start() {
        let s = sup("basic");
        let id = app(
            &s,
            "sleeper",
            "ping -n 60 127.0.0.1 >nul",
            RestartPolicy::OnFailure,
            5,
        );
        s.start(&id).unwrap();
        assert!(
            matches!(s.start(&id), Err(SupError::Conflict(_))),
            "second start must be refused"
        );
        assert_eq!(
            wait_for(&s, &id, AppState::Running, 5).await,
            AppState::Running
        );
        let pid = s.view(&id).unwrap().pid.unwrap();
        s.stop(&id).await.unwrap();
        assert_eq!(s.view(&id).unwrap().state, AppState::Stopped);
        assert!(
            job::process_name(pid).is_none(),
            "process {pid} still alive after stop"
        );
    }

    #[tokio::test]
    async fn concurrent_starts_run_one_process() {
        let s = sup("concurrent");
        let id = app(
            &s,
            "sleeper",
            "ping -n 60 127.0.0.1 >nul",
            RestartPolicy::OnFailure,
            5,
        );
        let (a, b) = (s.clone(), s.clone());
        let (id_a, id_b) = (id.clone(), id.clone());
        let ra = tokio::task::spawn_blocking(move || a.start(&id_a).is_ok());
        let rb = tokio::task::spawn_blocking(move || b.start(&id_b).is_ok());
        let (ra, rb) = (ra.await.unwrap(), rb.await.unwrap());
        assert!(ra ^ rb, "exactly one start must win (got {ra} and {rb})");
        s.stop(&id).await.unwrap();
        assert_eq!(s.view(&id).unwrap().state, AppState::Stopped);
    }

    #[tokio::test]
    async fn crash_loop_ends_in_failed_and_stop_cancels_restart() {
        let s = sup("crash");
        let id = app(&s, "crasher", "exit 3", RestartPolicy::OnFailure, 1);
        s.start(&id).unwrap();
        // Run 1 exits, restart 1 after 1s exits, then Failed.
        assert_eq!(
            wait_for(&s, &id, AppState::Failed, 10).await,
            AppState::Failed
        );
        assert_eq!(s.view(&id).unwrap().exit_code, Some(3));

        let id2 = app(&s, "crasher2", "exit 1", RestartPolicy::Always, 0);
        s.start(&id2).unwrap();
        assert_eq!(
            wait_for(&s, &id2, AppState::Restarting, 5).await,
            AppState::Restarting
        );
        s.stop(&id2).await.unwrap();
        tokio::time::sleep(Duration::from_millis(1500)).await;
        assert_eq!(
            s.view(&id2).unwrap().state,
            AppState::Stopped,
            "stopped app was restarted"
        );

        let kinds: Vec<EventKind> = s
            .events(Some(&id), 20)
            .unwrap()
            .iter()
            .map(|e| e.kind)
            .collect();
        assert!(kinds.contains(&EventKind::Failed) && kinds.contains(&EventKind::Restarted));
    }

    #[tokio::test]
    async fn clean_exit_is_not_restarted_on_failure_policy() {
        let s = sup("clean");
        let id = app(&s, "once", "exit 0", RestartPolicy::OnFailure, 5);
        s.start(&id).unwrap();
        assert_eq!(
            wait_for(&s, &id, AppState::Exited, 5).await,
            AppState::Exited
        );
    }

    #[test]
    fn backoff_doubles_and_caps() {
        assert_eq!(backoff(1), Duration::from_secs(1));
        assert_eq!(backoff(2), Duration::from_secs(2));
        assert_eq!(backoff(4), Duration::from_secs(8));
        assert_eq!(backoff(7), Duration::from_secs(60));
        assert_eq!(backoff(200), Duration::from_secs(60));
    }

    #[test]
    fn change_description() {
        let a: AppSpec = serde_json::from_str(
            r#"{"id":"a","name":"a","cwd":"C:\\","command":"x","env":{"A":"1"}}"#,
        )
        .unwrap();
        let mut b = a.clone();
        b.command = "y".into();
        b.env.insert("B".into(), "2".into());
        b.autostart = false;
        assert_eq!(
            describe_changes(&a, &b),
            "command, environment (B), start with agent to off"
        );
        assert!(needs_restart(&a, &b));
    }
}
