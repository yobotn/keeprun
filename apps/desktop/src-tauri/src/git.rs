//! Git commands for the UI. The work is in `keeprun_core::git` and runs as the signed-in user.

use keeprun_core::git::{GitStatus, Pulled};
use std::path::Path;

/// None when the folder is not in a Git repository or Git is not installed.
#[tauri::command(async)]
pub fn git_status(cwd: String) -> Option<GitStatus> {
    keeprun_core::git::status(Path::new(&cwd)).ok()
}

/// Fetch without asking for credentials, then the state.
#[tauri::command(async)]
pub fn git_fetch(cwd: String) -> Result<GitStatus, String> {
    keeprun_core::git::fetch(Path::new(&cwd))
}

/// Fast-forward only. The user clicked, so Git may ask for credentials.
#[tauri::command(async)]
pub fn git_pull(cwd: String) -> Result<Pulled, String> {
    keeprun_core::git::pull(Path::new(&cwd), true)
}
