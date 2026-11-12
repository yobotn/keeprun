//! Notification area icon. It shows the health of all apps as a colored dot on the app icon,
//! and keeps Keeprun running when the window is closed.
//!
//! An app that runs but prints errors, such as a dev server whose build broke, turns the dot
//! red and gets a "Copy fix prompt" menu item. This is silent, with no Windows notification,
//! and waits while a coding agent is still working in the app's folder: errors from half-done
//! edits usually go away. The details are in the app.
//!
//! Health is polled here, not in the webview: a hidden webview throttles its timers, and the
//! icon must stay current while the window is closed. The UI only sends its translated text.

use keeprun_core::model::{AppState, AppView, IssueReport};
use keeprun_core::Settings;
use serde::Deserialize;
use std::collections::{BTreeMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use tauri::image::Image;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_notification::NotificationExt;

const TRAY_ID: &str = "keeprun";
const POLL: Duration = Duration::from_secs(2);
const BASE_ICON: &[u8] = include_bytes!("../icons/32x32.png");
const ISSUE_ITEM: &str = "issue:";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Level {
    /// Nothing is running.
    Idle,
    Good,
    /// Apps are starting or stopping.
    Busy,
    /// An app crashed and is restarting, or exited with an error.
    Warning,
    /// An app failed or prints errors, or the agent cannot be reached.
    Bad,
}

impl Level {
    fn color(self) -> Option<[u8; 3]> {
        match self {
            Level::Idle => None,
            Level::Good => Some([0x22, 0xc5, 0x5e]),
            Level::Busy => Some([0xea, 0xb3, 0x08]),
            Level::Warning => Some([0xf9, 0x73, 0x16]),
            Level::Bad => Some([0xef, 0x44, 0x44]),
        }
    }
}

/// Translated text from the UI. Plural entries are already chosen per count by the UI.
#[derive(Deserialize, Clone)]
#[serde(rename_all = "camelCase", default)]
pub struct TrayText {
    show: String,
    quit: String,
    /// "{running} of {total} apps running"
    summary: String,
    no_apps: String,
    unreachable: String,
    /// Count to text, e.g. "1" -> "1 app failed". Covers counts up to the number of apps.
    failed: BTreeMap<u32, String>,
    restarting: BTreeMap<u32, String>,
    /// "1 app has errors"
    errors: BTreeMap<u32, String>,
    /// Menu item, "Copy fix prompt for {name}".
    copy_prompt: String,
    /// Notification after copying from the menu.
    copied: String,
}

impl Default for TrayText {
    fn default() -> Self {
        Self {
            show: "Show Keeprun".into(),
            quit: "Quit Keeprun".into(),
            summary: "{running} of {total} apps running".into(),
            no_apps: "No apps".into(),
            unreachable: "Agent not reachable".into(),
            failed: BTreeMap::new(),
            restarting: BTreeMap::new(),
            errors: BTreeMap::new(),
            copy_prompt: "Copy fix prompt for {name}".into(),
            copied: "Fix prompt copied. Paste it into your coding agent.".into(),
        }
    }
}

pub struct Tray {
    /// Close hides the window to the notification area. On by default, the UI syncs the choice.
    enabled: AtomicBool,
    text: Mutex<TrayText>,
    last: Mutex<Option<(Level, String)>>,
    /// Apps with an issue, as (id, name), in the current menu.
    issue_apps: Mutex<Vec<(String, String)>>,
    /// Apps that print errors while a coding agent still works in their folder.
    waiting: Mutex<Vec<String>>,
}

impl Default for Tray {
    fn default() -> Self {
        Self {
            enabled: AtomicBool::new(true),
            text: Mutex::default(),
            last: Mutex::default(),
            issue_apps: Mutex::default(),
            waiting: Mutex::default(),
        }
    }
}

impl Tray {
    pub fn enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }
}

/// A running app that prints errors. Failed or restarting apps count as such instead.
fn has_issue(a: &AppView) -> bool {
    a.issue.is_some()
        && !a.spec.disabled
        && matches!(a.state, AppState::Running | AppState::Starting)
}

/// Ids of apps whose errors show: no coding agent is still working in their folder.
fn settled_issues(apps: &[AppView]) -> HashSet<String> {
    apps.iter()
        .filter(|a| has_issue(a) && !keeprun_core::sessions::agent_working(&a.spec.cwd))
        .map(|a| a.spec.id.clone())
        .collect()
}

/// Count apps by health. Disabled apps do not count. Only apps in `settled` count as
/// printing errors.
fn assess(apps: &[AppView], settled: &HashSet<String>) -> (Level, usize, usize, u32, u32, u32) {
    let apps: Vec<_> = apps.iter().filter(|a| !a.spec.disabled).collect();
    let running = apps.iter().filter(|a| a.state == AppState::Running).count();
    let failed = apps.iter().filter(|a| a.state == AppState::Failed).count() as u32;
    let restarting = apps
        .iter()
        .filter(|a| {
            a.state == AppState::Restarting
                || (a.state == AppState::Exited && a.exit_code.is_some_and(|c| c != 0))
        })
        .count() as u32;
    let busy = apps
        .iter()
        .any(|a| matches!(a.state, AppState::Starting | AppState::Stopping));
    let issues = apps
        .iter()
        .filter(|a| has_issue(a) && settled.contains(&a.spec.id))
        .count() as u32;
    let level = if failed > 0 || issues > 0 {
        Level::Bad
    } else if restarting > 0 {
        Level::Warning
    } else if busy {
        Level::Busy
    } else if running > 0 {
        Level::Good
    } else {
        Level::Idle
    };
    (level, running, apps.len(), failed, restarting, issues)
}

fn tooltip(
    text: &TrayText,
    apps: Option<&[AppView]>,
    settled: &HashSet<String>,
) -> (Level, String) {
    let Some(apps) = apps else {
        return (Level::Bad, format!("Keeprun\n{}", text.unreachable));
    };
    let (level, running, total, failed, restarting, issues) = assess(apps, settled);
    let mut lines = vec!["Keeprun".to_string()];
    lines.push(if total == 0 {
        text.no_apps.clone()
    } else {
        text.summary
            .replace("{running}", &running.to_string())
            .replace("{total}", &total.to_string())
    });
    let pick =
        |map: &BTreeMap<u32, String>, n: u32| map.get(&n).cloned().unwrap_or_else(|| n.to_string());
    if failed > 0 {
        lines.push(pick(&text.failed, failed));
    }
    if restarting > 0 {
        lines.push(pick(&text.restarting, restarting));
    }
    if issues > 0 {
        lines.push(pick(&text.errors, issues));
    }
    // Windows cuts tooltips at 127 characters.
    let mut tip = lines.join("\n");
    if tip.chars().count() > 127 {
        tip = tip.chars().take(127).collect();
    }
    (level, tip)
}

/// The app icon with a status dot in the bottom right corner.
fn icon(level: Level) -> Image<'static> {
    let base = Image::from_bytes(BASE_ICON).expect("bundled tray icon is a valid PNG");
    let (w, h) = (base.width(), base.height());
    let mut rgba = base.rgba().to_vec();
    if let Some([r, g, b]) = level.color() {
        let size = w.min(h) as f32;
        let radius = size * 0.25;
        let ring = size * 0.06;
        let (cx, cy) = (size - radius - 0.5, size - radius - 0.5);
        for y in 0..h {
            for x in 0..w {
                let d = ((x as f32 + 0.5 - cx).powi(2) + (y as f32 + 0.5 - cy).powi(2)).sqrt();
                // Anti-aliased dot, with a dark ring so it reads on light and dark taskbars.
                let outer = (radius + ring + 0.5 - d).clamp(0.0, 1.0);
                if outer <= 0.0 {
                    continue;
                }
                let inner = (radius + 0.5 - d).clamp(0.0, 1.0);
                let i = ((y * w + x) * 4) as usize;
                let mix = |bg: u8, fg: u8, a: f32| (bg as f32 * (1.0 - a) + fg as f32 * a) as u8;
                let (ring_c, a) = ([0x18u8, 0x18, 0x1b], outer);
                for (k, c) in ring_c.iter().enumerate() {
                    rgba[i + k] = mix(rgba[i + k], *c, a);
                }
                for (k, c) in [r, g, b].iter().enumerate() {
                    rgba[i + k] = mix(rgba[i + k], *c, inner);
                }
                rgba[i + 3] = rgba[i + 3].max((outer * 255.0) as u8);
            }
        }
    }
    Image::new_owned(rgba, w, h)
}

/// Show and Quit, with a "Copy fix prompt" item first for each app that prints errors.
fn menu<R: Runtime>(
    app: &AppHandle<R>,
    text: &TrayText,
    issues: &[(String, String)],
) -> tauri::Result<Menu<R>> {
    let menu = Menu::new(app)?;
    for (id, name) in issues {
        let label = text.copy_prompt.replace("{name}", name);
        let id = format!("{ISSUE_ITEM}{id}");
        menu.append(&MenuItem::with_id(app, id, label, true, None::<&str>)?)?;
    }
    if !issues.is_empty() {
        menu.append(&PredefinedMenuItem::separator(app)?)?;
    }
    menu.append(&MenuItem::with_id(
        app,
        "show",
        &text.show,
        true,
        None::<&str>,
    )?)?;
    menu.append(&MenuItem::with_id(
        app,
        "quit",
        &text.quit,
        true,
        None::<&str>,
    )?)?;
    Ok(menu)
}

pub fn show_window<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

pub fn setup<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let text = TrayText::default();
    TrayIconBuilder::with_id(TRAY_ID)
        .icon(icon(Level::Idle))
        .tooltip("Keeprun")
        .menu(&menu(app, &text, &[])?)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, e| match e.id.as_ref() {
            "show" => show_window(app),
            "quit" => app.exit(0),
            id => {
                if let Some(app_id) = id.strip_prefix(ISSUE_ITEM) {
                    copy_prompt(app.clone(), app_id.to_string());
                }
            }
        })
        .on_tray_icon_event(|tray, e| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = e
            {
                show_window(tray.app_handle());
            }
        })
        .build(app)?;

    let handle = app.clone();
    tauri::async_runtime::spawn(async move {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap_or_default();
        loop {
            let apps = fetch(&http).await;
            refresh(&handle, apps.as_deref());
            tokio::time::sleep(POLL).await;
        }
    });
    Ok(())
}

async fn get<T: serde::de::DeserializeOwned>(http: &reqwest::Client, path: &str) -> Option<T> {
    let s = Settings::load().ok()?;
    let token = keeprun_core::token::read(&s.token_path()).ok()?;
    let res = http
        .get(format!("{}{path}", s.base_url()))
        .bearer_auth(token)
        .send()
        .await
        .ok()?;
    if !res.status().is_success() {
        return None;
    }
    res.json().await.ok()
}

async fn fetch(http: &reqwest::Client) -> Option<Vec<AppView>> {
    get(http, "/apps").await
}

/// Copy an app's fix prompt from the menu, and confirm with a notification.
fn copy_prompt<R: Runtime>(app: AppHandle<R>, id: String) {
    tauri::async_runtime::spawn(async move {
        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .unwrap_or_default();
        let report: Option<IssueReport> = get(&http, &format!("/apps/{id}/issue")).await;
        // None: the issue cleared since the menu was built. The next poll removes the item.
        let Some(prompt) = report.and_then(|r| r.prompt) else {
            return;
        };
        if app.clipboard().write_text(prompt).is_ok() {
            let body = app.state::<Tray>().text.lock().unwrap().copied.clone();
            let _ = app
                .notification()
                .builder()
                .title("Keeprun")
                .body(body)
                .show();
        }
    });
}

fn tray<R: Runtime>(app: &AppHandle<R>) -> Option<TrayIcon<R>> {
    app.tray_by_id(TRAY_ID)
}

/// Update the icon, tooltip and menu when they change.
fn refresh<R: Runtime>(app: &AppHandle<R>, apps: Option<&[AppView]>) {
    let state = app.state::<Tray>();
    let settled = settled_issues(apps.unwrap_or_default());
    *state.waiting.lock().unwrap() = apps
        .unwrap_or_default()
        .iter()
        .filter(|a| has_issue(a) && !settled.contains(&a.spec.id))
        .map(|a| a.spec.id.clone())
        .collect();
    let issues: Vec<(String, String)> = apps
        .unwrap_or_default()
        .iter()
        .filter(|a| settled.contains(&a.spec.id))
        .map(|a| (a.spec.id.clone(), a.spec.name.clone()))
        .collect();
    // Tray and menu calls run on the main thread and wait for it. Never hold a lock across
    // them: a command on the main thread that takes the same lock would wait for this thread
    // while it waits for the main thread, and the window would hang.
    let text = state.text.lock().unwrap().clone();
    let menu_changed = {
        let mut current = state.issue_apps.lock().unwrap();
        let changed = *current != issues;
        if changed {
            current.clone_from(&issues);
        }
        changed
    };
    if menu_changed {
        if let (Some(t), Ok(m)) = (tray(app), menu(app, &text, &issues)) {
            let _ = t.set_menu(Some(m));
        }
    }
    let (level, tip) = tooltip(&text, apps, &settled);
    let previous = {
        let mut last = state.last.lock().unwrap();
        if last.as_ref().is_some_and(|(l, t)| *l == level && *t == tip) {
            return;
        }
        last.replace((level, tip.clone())).map(|(l, _)| l)
    };
    if let Some(t) = tray(app) {
        if previous != Some(level) {
            let _ = t.set_icon(Some(icon(level)));
        }
        let _ = t.set_tooltip(Some(&tip));
    }
}

/// Ids of apps whose errors wait for a coding agent to finish, as of the last poll. The UI
/// holds their error banner back too.
#[tauri::command]
pub fn agent_waiting<R: Runtime>(app: AppHandle<R>) -> Vec<String> {
    app.state::<Tray>().waiting.lock().unwrap().clone()
}

/// Turn the notification area icon on or off. Off: closing the window quits the app.
#[tauri::command]
pub fn set_tray_enabled<R: Runtime>(app: AppHandle<R>, enabled: bool) {
    app.state::<Tray>()
        .enabled
        .store(enabled, Ordering::Relaxed);
    if let Some(t) = tray(&app) {
        let _ = t.set_visible(enabled);
    }
}

/// Translated menu and tooltip text, sent when the UI language changes.
#[tauri::command]
pub fn set_tray_text<R: Runtime>(app: AppHandle<R>, text: TrayText) {
    let state = app.state::<Tray>();
    let issues = state.issue_apps.lock().unwrap().clone();
    if let (Some(t), Ok(m)) = (tray(&app), menu(&app, &text, &issues)) {
        let _ = t.set_menu(Some(m));
    }
    *state.text.lock().unwrap() = text;
    // Rebuild the tooltip with the new text on the next poll.
    *state.last.lock().unwrap() = None;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> HashSet<String> {
        HashSet::new()
    }

    fn view(state: AppState, exit_code: Option<i32>, disabled: bool) -> AppView {
        serde_json::from_value(serde_json::json!({
            "id": "a", "name": "a", "cwd": "C:\\", "command": "x",
            "state": state, "pid": null, "exit_code": exit_code, "started_at": null,
            "next_restart_at": null, "stats": null, "disabled": disabled
        }))
        .unwrap()
    }

    #[test]
    fn levels() {
        use AppState::*;
        assert_eq!(assess(&[], &none()).0, Level::Idle);
        assert_eq!(
            assess(&[view(Stopped, None, false)], &none()).0,
            Level::Idle
        );
        assert_eq!(
            assess(&[view(Running, None, false)], &none()).0,
            Level::Good
        );
        assert_eq!(
            assess(
                &[view(Running, None, false), view(Starting, None, false)],
                &none()
            )
            .0,
            Level::Busy
        );
        assert_eq!(
            assess(
                &[view(Running, None, false), view(Restarting, Some(1), false)],
                &none()
            )
            .0,
            Level::Warning
        );
        assert_eq!(
            assess(&[view(Exited, Some(0), false)], &none()).0,
            Level::Idle
        );
        assert_eq!(
            assess(&[view(Exited, Some(3), false)], &none()).0,
            Level::Warning
        );
        assert_eq!(
            assess(
                &[view(Failed, Some(1), false), view(Restarting, None, false)],
                &none()
            )
            .0,
            Level::Bad
        );
        // Disabled apps never count.
        assert_eq!(
            assess(
                &[view(Failed, Some(1), true), view(Running, None, false)],
                &none()
            )
            .0,
            Level::Good
        );
        assert_eq!(
            assess(
                &[view(Running, None, false), view(Stopped, None, true)],
                &none()
            )
            .2,
            1
        );
    }

    #[test]
    fn errors_turn_red_once_settled() {
        let mut broken = view(AppState::Running, None, false);
        broken.spec.id = "b".into();
        broken.issue = Some(keeprun_core::model::AppIssue {
            first_at: 1,
            last_at: 1,
            count: 3,
            summary: "Error: x".into(),
            excerpt: vec![],
        });
        let apps = [view(AppState::Running, None, false), broken.clone()];
        let text = TrayText {
            errors: BTreeMap::from([(1, "1 app has errors".into())]),
            ..Default::default()
        };
        // A coding agent is still working in the folder: nothing shows yet.
        let (level, _, _, _, _, issues) = assess(&apps, &none());
        assert_eq!((level, issues), (Level::Good, 0));
        assert_eq!(
            tooltip(&text, Some(&apps), &none()).1,
            "Keeprun\n2 of 2 apps running"
        );
        // It finished and the errors are still there.
        let settled = HashSet::from(["b".to_string()]);
        let (level, _, _, _, _, issues) = assess(&apps, &settled);
        assert_eq!((level, issues), (Level::Bad, 1));
        assert_eq!(
            tooltip(&text, Some(&apps), &settled).1,
            "Keeprun\n2 of 2 apps running\n1 app has errors"
        );
        // A failed app is red, and not counted again as an issue.
        broken.state = AppState::Failed;
        let (level, _, _, failed, _, issues) = assess(&[broken], &settled);
        assert_eq!((level, failed, issues), (Level::Bad, 1, 0));
    }

    #[test]
    fn tooltip_text() {
        let text = TrayText {
            failed: BTreeMap::from([(1, "1 app failed".into())]),
            ..Default::default()
        };
        let apps = [
            view(AppState::Running, None, false),
            view(AppState::Failed, Some(1), false),
        ];
        let (level, tip) = tooltip(&text, Some(&apps), &none());
        assert_eq!(level, Level::Bad);
        assert_eq!(tip, "Keeprun\n1 of 2 apps running\n1 app failed");
        assert_eq!(
            tooltip(&text, None, &none()).1,
            "Keeprun\nAgent not reachable"
        );
    }

    #[test]
    fn icon_has_dot() {
        let plain = icon(Level::Idle);
        let red = icon(Level::Bad);
        assert_eq!(plain.width(), red.width());
        assert_ne!(plain.rgba(), red.rgba());
    }
}
