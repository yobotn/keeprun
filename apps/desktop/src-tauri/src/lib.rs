//! Keeprun desktop shell. The UI is a pure client of the agent API. Rust resolves which agent
//! to talk to (from the active environment), hands the token to the webview, and runs the
//! detector in the user's context so per-user tool installs are found.

use keeprun_core::detect::Detection;
use keeprun_core::{Env, Settings};
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use tauri::Manager;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Connection {
    env: Env,
    base_url: String,
    ws_url: String,
    token: Option<String>,
    /// Set when the token cannot be read, with a message for the user.
    error: Option<String>,
    data_dir: String,
    version: String,
}

/// Re-read on every call so the UI picks up a token created after it started.
#[tauri::command]
fn agent_connection() -> Result<Connection, String> {
    let s = Settings::load().map_err(|e| format!("{e:#}"))?;
    let (token, error) = match keeprun_core::token::read(&s.token_path()) {
        Ok(t) => (Some(t), None),
        Err(e) => (None, Some(format!("{e:#}"))),
    };
    Ok(Connection {
        env: s.env,
        base_url: s.base_url(),
        ws_url: format!("ws://{}:{}", s.host, s.port),
        token,
        error,
        data_dir: s.data_dir.display().to_string(),
        version: env!("CARGO_PKG_VERSION").into(),
    })
}

#[tauri::command]
fn detect(path: PathBuf) -> Result<Detection, String> {
    keeprun_core::detect::detect(&path)
}

#[tauri::command]
fn read_env_file(path: PathBuf) -> Result<BTreeMap<String, String>, String> {
    let text = std::fs::read_to_string(&path)
        .map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    keeprun_core::model::parse_dotenv(&text)
}

#[tauri::command]
fn looks_secret(key: String) -> bool {
    keeprun_core::model::looks_secret(&key)
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            if let (Ok(env), Some(w)) = (Env::detect(), app.get_webview_window("main")) {
                if env == Env::Dev {
                    let _ = w.set_title("Keeprun (dev)");
                }
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            agent_connection,
            detect,
            read_env_file,
            looks_secret
        ])
        .run(tauri::generate_context!())
        .expect("error while running Keeprun");
}
