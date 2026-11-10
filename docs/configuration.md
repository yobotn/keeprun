# Configuration and environments

Keeprun has two environments, `dev` and `prod`. They use different ports, data folders and
service names, so a dev agent and the installed service can run side by side.

|                  | dev                              | prod                                   |
|------------------|----------------------------------|----------------------------------------|
| Selected by      | debug builds, or `KEEPRUN_ENV=dev` | release builds, or `KEEPRUN_ENV=prod`  |
| API port         | 7181                             | 7171                                   |
| Data folder      | `<repo>\.dev-data`               | `C:\ProgramData\Keeprun`               |
| Config file      | `config/dev.toml`, then `config/dev.local.toml` | `C:\ProgramData\Keeprun\config.toml` |
| Log level        | `keeprun=debug,info`             | `keeprun=info,warn`                    |
| Stop timeout     | 3 s                              | 10 s                                   |
| Service name     | `KeeprunAgentDev`                | `KeeprunAgent`                         |
| UI origins (CORS)| Tauri + Vite dev server (:1420)  | Tauri only                             |
| UI               | "(dev)" title, DEV badge, `.env.development` | `.env.production`          |

Every binary takes `--env dev|prod`, which wins over `KEEPRUN_ENV`.

## Resolution order

Later wins:

1. Built-in defaults for the environment.
2. The config file, or the file named by `KEEPRUN_CONFIG`.
3. Environment variables:

| Variable | Setting |
|---|---|
| `KEEPRUN_ENV` | `dev` or `prod` |
| `KEEPRUN_CONFIG` | Path to a config file |
| `KEEPRUN_HOST` | Bind address. Keep `127.0.0.1`: the API is local only. |
| `KEEPRUN_PORT` | API port |
| `KEEPRUN_DATA_DIR` | Database, token and logs |
| `KEEPRUN_LOG` | Agent log filter, e.g. `keeprun=debug` |
| `KEEPRUN_STOP_TIMEOUT_MS` | Wait after Ctrl+Break before ending the process tree |

Print what the agent will use:

```powershell
keeprun-agent config --env prod
```

## Data folder

| Path | Content |
|---|---|
| `keeprun.db` | Apps, desired state, events (SQLite). Secret env values are DPAPI encrypted. |
| `agent.token` | API token |
| `agent.log` | Agent log when running as a service |
| `logs\<app>\current.log` | App output, rotated to `previous.log` at 10 MB |
| `config.toml` | Prod settings |

## Desktop UI settings

`apps/desktop/.env.development` and `.env.production`:

| Variable | Default (dev / prod) | Meaning |
|---|---|---|
| `VITE_STATUS_POLL_MS` | 1000 / 2000 | Status refresh interval |
| `VITE_LOG_MAX_LINES` | 5000 / 10000 | Lines kept in the log view |
