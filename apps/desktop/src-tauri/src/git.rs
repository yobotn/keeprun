//! Git commands for the UI. The work is in `keeprun_core::git` and runs as the signed-in user.

use keeprun_core::git::{GitStatus, Pulled};
use keeprun_core::model::TaskSpec;
use std::path::Path;

/// None when the folder is not in a Git repository or Git is not installed.
#[tauri::command(async)]
pub fn git_status(cwd: String) -> Option<GitStatus> {
    keeprun_core::git::status(Path::new(&cwd)).ok()
}

/// Fast-forward only. The user clicked, so Git may ask for credentials.
#[tauri::command(async)]
pub fn git_pull<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    cwd: String,
) -> Result<Pulled, String> {
    use tauri::Manager;
    let pulled = keeprun_core::git::pull(Path::new(&cwd), true)?;
    app.state::<crate::remotes::Remotes>()
        .pulled(&cwd, &pulled.status);
    Ok(pulled)
}

/// The saved task that applies database migrations, as `keeprun migrate` picks it. Pull runs
/// it when migrations changed.
#[tauri::command]
pub fn migrate_task(tasks: Vec<TaskSpec>) -> Option<String> {
    keeprun_core::migrate::task(&tasks).map(|t| t.name.clone())
}
