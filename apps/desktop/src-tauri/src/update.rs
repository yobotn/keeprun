//! Desktop update check and install, through the Tauri updater.
//!
//! An update replaces the whole install (desktop app, agent service, CLI) by running the signed
//! MSI or NSIS installer, so it needs administrator approval. IT can turn checks off or point
//! them at an internal mirror with policy values under `HKLM\SOFTWARE\Policies\Keeprun`:
//!
//! - `DisableUpdates` (DWORD 1): no checks, no update button.
//! - `UpdateUrl` (string): manifest URL to use instead of the public one.

use keeprun_core::{Env, Settings};
use serde::Serialize;
use std::sync::Mutex;
use tauri::ipc::Channel;
use tauri::{AppHandle, State};
use tauri_plugin_updater::{Update, UpdaterExt};

const POLICY_KEY: &str = r"SOFTWARE\Policies\Keeprun";

#[derive(Default)]
pub struct Pending(Mutex<Option<Update>>);

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Policy {
    enabled: bool,
    /// Why updates are off, for the UI.
    reason: Option<String>,
    url: Option<String>,
}

fn policy() -> Policy {
    use winreg::enums::HKEY_LOCAL_MACHINE;
    use winreg::RegKey;
    if Settings::load().map(|s| s.env == Env::Dev).unwrap_or(false) {
        return Policy {
            enabled: false,
            reason: Some("Updates are off in the dev environment.".into()),
            url: None,
        };
    }
    let key = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(POLICY_KEY)
        .ok();
    let disabled = key
        .as_ref()
        .and_then(|k| k.get_value::<u32, _>("DisableUpdates").ok())
        .is_some_and(|v| v != 0);
    let url = key
        .as_ref()
        .and_then(|k| k.get_value::<String, _>("UpdateUrl").ok())
        .map(|u| u.trim().to_string())
        .filter(|u| !u.is_empty());
    Policy {
        enabled: !disabled,
        reason: disabled.then(|| "Updates are managed by your IT department.".into()),
        url,
    }
}

#[tauri::command]
pub fn update_policy() -> Policy {
    policy()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    version: String,
    current_version: String,
    notes: Option<String>,
    /// RFC 3339.
    date: Option<String>,
}

#[tauri::command]
pub async fn check_update(
    app: AppHandle,
    pending: State<'_, Pending>,
) -> Result<Option<UpdateInfo>, String> {
    let p = policy();
    if !p.enabled {
        return Ok(None);
    }
    let mut builder = app.updater_builder();
    if let Some(url) = p.url {
        let url = url
            .parse()
            .map_err(|_| format!("UpdateUrl policy \"{url}\" is not a valid URL."))?;
        builder = builder.endpoints(vec![url]).map_err(|e| e.to_string())?;
    }
    let update = builder
        .build()
        .map_err(|e| e.to_string())?
        .check()
        .await
        .map_err(|e| format!("Cannot check for updates: {e}"))?;
    let info = update.as_ref().map(|u| UpdateInfo {
        version: u.version.clone(),
        current_version: u.current_version.clone(),
        notes: u.body.clone(),
        date: u.date.and_then(|d| {
            d.format(&time::format_description::well_known::Rfc3339)
                .ok()
        }),
    });
    *pending.0.lock().unwrap() = update;
    Ok(info)
}

#[derive(Clone, Serialize)]
#[serde(tag = "event", content = "data", rename_all = "camelCase")]
pub enum Progress {
    Started { total: Option<u64> },
    Chunk { downloaded: u64 },
    Installing,
}

/// Download, verify the signature and run the installer. Windows closes the app while the
/// installer runs; the installer restarts the agent service.
#[tauri::command]
pub async fn install_update(
    pending: State<'_, Pending>,
    on_progress: Channel<Progress>,
) -> Result<(), String> {
    let update = pending
        .0
        .lock()
        .unwrap()
        .take()
        .ok_or("No update to install. Check for updates first.")?;
    let mut downloaded = 0u64;
    let mut started = false;
    update
        .download_and_install(
            |chunk, total| {
                if !started {
                    started = true;
                    let _ = on_progress.send(Progress::Started { total });
                }
                downloaded += chunk as u64;
                let _ = on_progress.send(Progress::Chunk { downloaded });
            },
            || {
                let _ = on_progress.send(Progress::Installing);
            },
        )
        .await
        .map_err(|e| format!("Update failed: {e}"))
}
