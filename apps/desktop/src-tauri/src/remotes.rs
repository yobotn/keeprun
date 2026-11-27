//! Background Git checks, off unless the user turns them on in Preferences. Every few minutes,
//! also while the window is closed, fetch each app's repository and tell the user when a remote
//! branch has commits to pull. Only fetches: the working tree never changes here.
//!
//! This runs in the desktop shell, as the signed-in user, never in the agent: the service
//! account has none of the user's Git credentials.

use keeprun_core::git::{GitStatus, Sync};
use keeprun_core::model::AppView;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, Runtime};
use tauri_plugin_notification::NotificationExt;

/// The first check waits for startup to settle.
const FIRST: Duration = Duration::from_secs(60);
const TICK: Duration = Duration::from_secs(30);

/// Notification text from the UI, in its language.
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct RemoteText {
    /// "{name} has new commits"
    title: String,
    /// "{count} new commits on {upstream}. Open Keeprun to pull."
    body: String,
}

impl Default for RemoteText {
    fn default() -> Self {
        Self {
            title: "{name} has new commits".into(),
            body: "New commits on {upstream}: {count}. Open Keeprun to pull.".into(),
        }
    }
}

/// The last fetch of a folder, sent to the UI after each one.
#[derive(Serialize, Clone)]
pub struct Checked {
    pub cwd: String,
    /// Unix time in milliseconds of the last successful fetch.
    pub at: Option<u64>,
    /// Why the last fetch failed. None when it worked.
    pub error: Option<String>,
    pub status: Option<GitStatus>,
}

pub struct Remotes {
    /// Minutes between checks. 0: only when the user refreshes.
    minutes: AtomicU32,
    notify: AtomicBool,
    text: Mutex<RemoteText>,
    /// Folders being fetched now. Two fetches of one repository fight over its lock files.
    busy: Mutex<HashSet<String>>,
    /// Last fetch of each folder.
    checked: Mutex<HashMap<String, Checked>>,
    /// Commits behind the user was last told about, per folder. A notification goes out only
    /// when there are more.
    told: Mutex<HashMap<String, u32>>,
}

impl Default for Remotes {
    fn default() -> Self {
        Self {
            // Off until the UI sends the user's choice: a fetch writes in the project's .git.
            minutes: AtomicU32::new(0),
            notify: AtomicBool::new(true),
            text: Mutex::default(),
            busy: Mutex::default(),
            checked: Mutex::default(),
            told: Mutex::default(),
        }
    }
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

impl Remotes {
    /// Fetch `cwd`, or wait for the fetch already running and read the state after it.
    /// Blocking: call from a worker thread.
    pub fn fetch(&self, cwd: &str) -> Result<GitStatus, String> {
        let mut waited = false;
        loop {
            {
                let mut busy = self.busy.lock().unwrap();
                if !busy.contains(cwd) {
                    if waited {
                        drop(busy);
                        return self.after_wait(cwd);
                    }
                    busy.insert(cwd.to_string());
                    break;
                }
            }
            waited = true;
            std::thread::sleep(Duration::from_millis(200));
        }
        let result = keeprun_core::git::fetch(Path::new(cwd));
        self.busy.lock().unwrap().remove(cwd);
        let mut checked = self.checked.lock().unwrap();
        let last_at = checked.get(cwd).and_then(|c| c.at);
        let entry = match &result {
            Ok(s) => Checked {
                cwd: cwd.to_string(),
                at: Some(now_ms()),
                error: None,
                status: Some(s.clone()),
            },
            Err(e) => Checked {
                cwd: cwd.to_string(),
                at: last_at,
                error: Some(e.clone()),
                status: checked.get(cwd).and_then(|c| c.status.clone()),
            },
        };
        checked.insert(cwd.to_string(), entry);
        result
    }

    /// The fetch someone else ran: its error, or the state now.
    fn after_wait(&self, cwd: &str) -> Result<GitStatus, String> {
        if let Some(e) = self
            .checked
            .lock()
            .unwrap()
            .get(cwd)
            .and_then(|c| c.error.clone())
        {
            return Err(e);
        }
        keeprun_core::git::status(Path::new(cwd))
    }

    /// A pull fetched and brought the branch up to date.
    pub fn pulled(&self, cwd: &str, status: &GitStatus) {
        self.told.lock().unwrap().insert(cwd.to_string(), 0);
        self.checked.lock().unwrap().insert(
            cwd.to_string(),
            Checked {
                cwd: cwd.to_string(),
                at: Some(now_ms()),
                error: None,
                status: Some(status.clone()),
            },
        );
    }

    fn checked(&self, cwd: &str) -> Option<Checked> {
        self.checked.lock().unwrap().get(cwd).cloned()
    }
}

/// How many new commits to tell the user about, if any: more than last time.
fn news(told: &mut HashMap<String, u32>, cwd: &str, s: &GitStatus) -> Option<u32> {
    let behind = match s.sync {
        Sync::Behind | Sync::Diverged => s.behind,
        _ => 0,
    };
    let before = told.insert(cwd.to_string(), behind).unwrap_or(0);
    (behind > before).then_some(behind)
}

/// Apps worth checking: enabled, one per folder.
fn folders(apps: &[AppView]) -> Vec<(String, String)> {
    let mut seen = HashSet::new();
    apps.iter()
        .filter(|a| !a.spec.disabled)
        .filter_map(|a| {
            let cwd = a.spec.cwd.to_string_lossy().to_string();
            seen.insert(cwd.to_lowercase())
                .then(|| (cwd, a.spec.name.clone()))
        })
        .collect()
}

pub fn setup<R: Runtime>(app: &AppHandle<R>) {
    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap_or_default();
        tokio::time::sleep(FIRST).await;
        let mut last: Option<Instant> = None;
        loop {
            let minutes = handle.state::<Remotes>().minutes.load(Ordering::Relaxed);
            let due = minutes > 0
                && last.is_none_or(|t| t.elapsed() >= Duration::from_secs(minutes as u64 * 60));
            if due {
                last = Some(Instant::now());
                if let Some(apps) = crate::tray::list_apps(&http).await {
                    check_all(&handle, folders(&apps)).await;
                }
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

async fn check_all<R: Runtime>(app: &AppHandle<R>, folders: Vec<(String, String)>) {
    for (cwd, name) in folders {
        let handle = app.clone();
        let dir = cwd.clone();
        let result =
            tauri::async_runtime::spawn_blocking(move || handle.state::<Remotes>().fetch(&dir))
                .await;
        let Ok(result) = result else { continue };
        let state = app.state::<Remotes>();
        let Some(checked) = state.checked(&cwd) else {
            continue;
        };
        // Not a Git repository, or no Git: nothing to say, now or later.
        if result.as_ref().is_err_and(|e| e == "no-git") || checked.status.is_none() {
            continue;
        }
        let _ = app.emit("git-checked", &checked);
        let Ok(status) = result else { continue };
        let count = news(&mut state.told.lock().unwrap(), &cwd, &status);
        if let (Some(count), true) = (count, state.notify.load(Ordering::Relaxed)) {
            let text = state.text.lock().unwrap().clone();
            let upstream = status.upstream.as_deref().unwrap_or("");
            let _ = app
                .notification()
                .builder()
                .title(text.title.replace("{name}", &name))
                .body(
                    text.body
                        .replace("{count}", &count.to_string())
                        .replace("{upstream}", upstream),
                )
                .show();
        }
    }
}

/// Fetch for the UI: the refresh button, or opening an app when background checks are on.
/// Never asks for credentials.
#[tauri::command(async)]
pub fn git_fetch<R: Runtime>(app: AppHandle<R>, cwd: String) -> Result<GitStatus, String> {
    let state = app.state::<Remotes>();
    let result = state.fetch(&cwd);
    if let Ok(s) = &result {
        // The user sees it now: do not notify about these commits again.
        let behind = if matches!(s.sync, Sync::Behind | Sync::Diverged) {
            s.behind
        } else {
            0
        };
        state.told.lock().unwrap().insert(cwd.clone(), behind);
    }
    if let Some(c) = state.checked(&cwd) {
        let _ = app.emit("git-checked", &c);
    }
    result
}

/// The last fetch of a folder, for the UI to show when it happened.
#[tauri::command]
pub fn git_checked<R: Runtime>(app: AppHandle<R>, cwd: String) -> Option<Checked> {
    app.state::<Remotes>().checked(&cwd)
}

/// Commits to pull per folder, as of the last fetch. Folders with none are left out.
#[tauri::command]
pub fn git_behind<R: Runtime>(app: AppHandle<R>) -> HashMap<String, u32> {
    app.state::<Remotes>()
        .checked
        .lock()
        .unwrap()
        .iter()
        .filter_map(|(cwd, c)| {
            let s = c.status.as_ref()?;
            (matches!(s.sync, Sync::Behind | Sync::Diverged) && s.behind > 0)
                .then(|| (cwd.clone(), s.behind))
        })
        .collect()
}

/// The user's choices and the notification text in their language.
#[tauri::command]
pub fn set_git_watch<R: Runtime>(app: AppHandle<R>, minutes: u32, notify: bool, text: RemoteText) {
    let state = app.state::<Remotes>();
    state.minutes.store(minutes, Ordering::Relaxed);
    state.notify.store(notify, Ordering::Relaxed);
    *state.text.lock().unwrap() = text;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn behind(n: u32) -> GitStatus {
        GitStatus {
            behind: n,
            sync: if n > 0 { Sync::Behind } else { Sync::InSync },
            ..Default::default()
        }
    }

    #[test]
    fn notifies_only_about_more_commits() {
        let mut told = HashMap::new();
        assert_eq!(news(&mut told, "a", &behind(0)), None);
        assert_eq!(news(&mut told, "a", &behind(2)), Some(2));
        // Same commits on the next check: no second notification.
        assert_eq!(news(&mut told, "a", &behind(2)), None);
        assert_eq!(news(&mut told, "a", &behind(3)), Some(3));
        // Pulled, then new commits again.
        assert_eq!(news(&mut told, "a", &behind(0)), None);
        assert_eq!(news(&mut told, "a", &behind(1)), Some(1));
        // Merged or gone branches have nothing to pull.
        let merged = GitStatus {
            sync: Sync::Merged,
            ..Default::default()
        };
        assert_eq!(news(&mut told, "b", &merged), None);
    }
}
