//! Windows service install, uninstall and runtime.

use anyhow::{Context, Result};
use keeprun_core::{Env, Settings};
use std::ffi::OsString;
use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use windows_service::service::{
    ServiceAccess, ServiceAction, ServiceActionType, ServiceControl, ServiceControlAccept,
    ServiceErrorControl, ServiceExitCode, ServiceFailureActions, ServiceFailureResetPeriod,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

static SETTINGS: OnceLock<Settings> = OnceLock::new();

define_windows_service!(ffi_service_main, service_main);

pub fn run_dispatcher(settings: Settings) -> Result<()> {
    let name = settings.service_name();
    let _ = SETTINGS.set(settings);
    service_dispatcher::start(name, ffi_service_main)
        .context("Cannot connect to the Service Control Manager. Use `keeprun-agent run` to run in a terminal.")
}

fn service_main(_args: Vec<OsString>) {
    let Some(settings) = SETTINGS.get().cloned() else {
        return;
    };
    if let Err(e) = run_service(settings) {
        tracing::error!("service failed: {e:#}");
    }
}

fn run_service(settings: Settings) -> Result<()> {
    crate::init_tracing(&settings, true)?;
    crate::job::ensure_hidden_console();
    let name = settings.service_name();

    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
    let tx = std::sync::Mutex::new(Some(tx));
    let handle = service_control_handler::register(name, move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            if let Some(tx) = tx.lock().unwrap().take() {
                let _ = tx.send(());
            }
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate => ServiceControlHandlerResult::NoError,
        _ => ServiceControlHandlerResult::NotImplemented,
    })?;

    let status = |state: ServiceState, accept: ServiceControlAccept, code: u32, wait: Duration| {
        ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: ServiceExitCode::Win32(code),
            checkpoint: 0,
            wait_hint: wait,
            process_id: None,
        }
    };

    // Every exit path below reports Stopped, so the SCM never shows a dead agent as running.
    let result = tokio::runtime::Runtime::new()
        .map_err(anyhow::Error::from)
        .and_then(|rt| {
            handle.set_service_status(status(
                ServiceState::Running,
                ServiceControlAccept::STOP | ServiceControlAccept::SHUTDOWN,
                0,
                Duration::default(),
            ))?;
            rt.block_on(crate::serve(Arc::new(settings), async move {
                let _ = rx.await;
            }))
        });

    // ERROR_SERVICE_SPECIFIC_ERROR style exit: 1 means the agent failed. Details are in agent.log.
    let code = if result.is_ok() { 0 } else { 1 };
    handle.set_service_status(status(
        ServiceState::Stopped,
        ServiceControlAccept::empty(),
        code,
        Duration::default(),
    ))?;
    result
}

pub fn install(settings: &Settings, start: bool, grant_user: Option<&str>) -> Result<()> {
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )
    .context(
        "Cannot open the Service Control Manager. Run this command from an Administrator terminal.",
    )?;

    std::fs::create_dir_all(&settings.data_dir)
        .with_context(|| format!("Cannot create {}", settings.data_dir.display()))?;
    if settings.env == Env::Prod {
        let cfg = settings.data_dir.join("config.toml");
        if !cfg.exists() {
            std::fs::write(&cfg, include_str!("../../../config/prod.toml"))?;
            println!("Wrote default config to {}", cfg.display());
        }
    }

    let token_path = settings.token_path();
    keeprun_core::token::load_or_create(&token_path)?;
    ensure_users_group();
    // The UI runs non-elevated, where UAC hides Administrators membership, so the user who
    // installs gets read access directly (works at once) and through the group (survives
    // reinstall and can be managed with `net localgroup`).
    let user = grant_user
        .map(str::to_string)
        .or_else(current_user)
        .filter(|u| !is_machine_account(u));
    if let Some(u) = &user {
        add_to_users_group(u);
    }
    secure_token_file(&token_path, user.as_deref())?;

    let exe = std::env::current_exe()?;
    let info = ServiceInfo {
        name: settings.service_name().into(),
        display_name: match settings.env {
            Env::Prod => "Keeprun Agent".into(),
            Env::Dev => "Keeprun Agent (dev)".into(),
        },
        service_type: ServiceType::OWN_PROCESS,
        start_type: ServiceStartType::AutoStart,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe,
        launch_arguments: vec![
            "service".into(),
            "--env".into(),
            settings.env.as_str().into(),
        ],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    };
    let access = ServiceAccess::CHANGE_CONFIG | ServiceAccess::START | ServiceAccess::QUERY_STATUS;
    let service = match manager.open_service(settings.service_name(), access) {
        Ok(existing) => {
            // Upgrade or repair: keep the service, refresh its configuration.
            existing
                .change_config(&info)
                .context("Cannot update the existing service")?;
            println!(
                "Updated service {} ({} environment).",
                settings.service_name(),
                settings.env
            );
            existing
        }
        Err(_) => {
            let created = manager
                .create_service(&info, access)
                .with_context(|| format!("Cannot create service {}.", settings.service_name()))?;
            println!(
                "Installed service {} ({} environment).",
                settings.service_name(),
                settings.env
            );
            created
        }
    };
    let _ = service
        .set_description("Runs and supervises internal apps. Managed by the Keeprun UI and CLI.");

    // If the agent itself crashes, Windows restarts it: 5s, 10s, then every 30s.
    let restart = |s| ServiceAction {
        action_type: ServiceActionType::Restart,
        delay: Duration::from_secs(s),
    };
    let actions = ServiceFailureActions {
        reset_period: ServiceFailureResetPeriod::After(Duration::from_secs(24 * 3600)),
        reboot_msg: None,
        command: None,
        actions: Some(vec![restart(5), restart(10), restart(30)]),
    };
    if let Err(e) = service.update_failure_actions(actions) {
        eprintln!("Warning: could not set service recovery actions: {e}");
    }
    let _ = service.set_failure_actions_on_non_crash_failures(true);

    // Installed builds put the CLI next to the agent. Make `keeprun` work in any new terminal.
    if settings.env == Env::Prod {
        if let Some(dir) = std::env::current_exe()?
            .parent()
            .filter(|d| d.join("keeprun.exe").is_file())
        {
            match crate::syspath::add(dir) {
                Ok(true) => println!(
                    "Added {} to PATH. Open a new terminal to use `keeprun`.",
                    dir.display()
                ),
                Ok(false) => {}
                Err(e) => eprintln!("Warning: {e:#}"),
            }
        }
    }

    if start && service.query_status()?.current_state == ServiceState::Stopped {
        service
            .start::<&str>(&[])
            .context("Service installed but did not start. Check agent.log in the data folder.")?;
        println!("Started. API at {}", settings.base_url());
    }
    Ok(())
}

const USERS_GROUP: &str = "Keeprun Users";

fn current_user() -> Option<String> {
    let u = std::env::var("USERNAME").ok()?;
    Some(match std::env::var("USERDOMAIN") {
        Ok(d) => format!(r"{d}\{u}"),
        Err(_) => u,
    })
}

/// SYSTEM and computer accounts (`NAME$`) appear when an MSI runs the install as LocalSystem.
fn is_machine_account(u: &str) -> bool {
    u.ends_with('$')
        || u.eq_ignore_ascii_case("SYSTEM")
        || u.to_ascii_uppercase().ends_with(r"\SYSTEM")
}

fn net(args: &[&str]) -> bool {
    std::process::Command::new("net")
        .args(args)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

fn ensure_users_group() {
    if !net(&["localgroup", USERS_GROUP]) {
        net(&[
            "localgroup",
            USERS_GROUP,
            "/add",
            "/comment:Can use the Keeprun desktop app and CLI.",
        ]);
    }
}

fn add_to_users_group(user: &str) {
    // Fails harmlessly when the user is already a member.
    net(&["localgroup", USERS_GROUP, user, "/add"]);
}

pub fn uninstall(settings: &Settings) -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
        .context(
        "Cannot open the Service Control Manager. Run this command from an Administrator terminal.",
    )?;
    let service = manager.open_service(
        settings.service_name(),
        ServiceAccess::QUERY_STATUS | ServiceAccess::STOP | ServiceAccess::DELETE,
    );
    let Ok(service) = service else {
        println!(
            "Service {} is not installed. Nothing to remove.",
            settings.service_name()
        );
        return Ok(());
    };

    if service.query_status()?.current_state != ServiceState::Stopped {
        service.stop()?;
        for _ in 0..60 {
            std::thread::sleep(Duration::from_millis(500));
            if service.query_status()?.current_state == ServiceState::Stopped {
                break;
            }
        }
    }
    service.delete()?;
    if settings.env == Env::Prod {
        if let Some(dir) = std::env::current_exe()?.parent() {
            let _ = crate::syspath::remove(dir);
        }
    }
    println!(
        "Removed service {}. Data in {} was kept.",
        settings.service_name(),
        settings.data_dir.display()
    );
    Ok(())
}

/// Limit the token file to SYSTEM, Administrators, the Keeprun Users group and, optionally,
/// one extra user (the UI user).
pub fn secure_token_file(path: &Path, extra_user: Option<&str>) -> Result<()> {
    let mut args: Vec<String> = vec![
        path.display().to_string(),
        "/inheritance:r".into(),
        "/grant:r".into(),
        "*S-1-5-18:F".into(),
        "/grant:r".into(),
        "*S-1-5-32-544:F".into(),
    ];
    if net(&["localgroup", USERS_GROUP]) {
        args.push("/grant:r".into());
        args.push(format!("{USERS_GROUP}:R"));
    }
    if let Some(u) = extra_user {
        args.push("/grant:r".into());
        args.push(format!("{u}:R"));
    }
    // A token readable by every local user would give them full control of the agent.
    let o = std::process::Command::new("icacls")
        .args(&args)
        .output()
        .context("Cannot run icacls")?;
    if !o.status.success() {
        anyhow::bail!(
            "Cannot restrict access to {}: {}",
            path.display(),
            String::from_utf8_lossy(&o.stdout).trim()
        );
    }
    Ok(())
}
