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

/// Change the agent port by running `keeprun-agent set-port`. The installed agent needs
/// Administrator rights to save the config and restart the service, so in prod Windows shows
/// a UAC prompt. The elevated process writes its outcome to a report file.
#[tauri::command]
async fn set_agent_port(port: u16) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || set_port_blocking(port))
        .await
        .map_err(|e| e.to_string())?
}

fn set_port_blocking(port: u16) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    let s = Settings::load().map_err(|e| format!("{e:#}"))?;
    let agent = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("keeprun-agent.exe")))
        .filter(|p| p.is_file())
        .ok_or("keeprun-agent.exe was not found next to the desktop app.")?;

    if s.env == Env::Dev {
        let out = std::process::Command::new(&agent)
            .args(["set-port", &port.to_string(), "--env", "dev"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return if out.status.success() {
            Ok(text)
        } else {
            Err(err.trim_start_matches("Error: ").to_string())
        };
    }

    let report = std::env::temp_dir().join(format!("keeprun-set-port-{}.txt", std::process::id()));
    let _ = std::fs::remove_file(&report);
    let q = |p: &std::path::Path| p.display().to_string().replace('\'', "''");
    let script = format!(
        "$p = Start-Process -FilePath '{}' -ArgumentList 'set-port','{port}','--env','prod','--report','\"{}\"' -Verb RunAs -WindowStyle Hidden -Wait -PassThru; exit $p.ExitCode",
        q(&agent),
        q(&report)
    );
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| e.to_string())?;
    let result = std::fs::read_to_string(&report).ok();
    let _ = std::fs::remove_file(&report);
    match result.as_deref().map(str::trim) {
        Some("ok") => Ok(format!(
            "The agent now listens on http://{}:{port}.",
            s.host
        )),
        Some(msg) => Err(msg.to_string()),
        None if !out.status.success()
            && String::from_utf8_lossy(&out.stderr).contains("canceled") =>
        {
            Err("Administrator approval was cancelled. The port was not changed.".into())
        }
        None => Err(format!(
            "The port was not changed. Run in an Administrator terminal: keeprun-agent set-port {port}"
        )),
    }
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
            looks_secret,
            set_agent_port
        ])
        .run(tauri::generate_context!())
        .expect("error while running Keeprun");
}
