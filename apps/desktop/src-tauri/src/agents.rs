//! A note in an app's CLAUDE.md or AGENTS.md that tells coding agents the app runs in Keeprun.
//! The note itself lives in keeprun-core, shared with `keeprun agent-setup`.

use keeprun_core::agent_note;
use std::path::PathBuf;

/// Name of the file in the app folder that tells coding agents to use Keeprun, if any.
#[tauri::command(async)]
pub fn agent_note_file(cwd: PathBuf) -> Option<String> {
    agent_note::noted(&cwd).map(str::to_string)
}

/// Add the note to CLAUDE.md, or to AGENTS.md when only that exists, or update an older note.
/// Returns the file name.
#[tauri::command(async)]
pub fn add_agent_note(cwd: PathBuf, app_id: String) -> Result<String, String> {
    agent_note::write(&cwd, &app_id).map(|w| w.file().to_string())
}
