# Keeprun

Run and manage internal apps on Windows. Pick a folder, click Run, and it stays up.

Keeprun runs Node, .NET, Python and custom apps as supervised processes under a Windows
service, with a desktop app and a CLI. No Docker, WSL or Hyper-V needed.

- Detects the stack and suggests the command, with full paths that work under LocalSystem.
- Every app runs in its own Job Object: Stop ends the whole process tree, nothing is left behind.
- Restarts crashed apps with backoff, starts them again after a reboot, marks them Failed
  when they keep crashing.
- Live logs with search, environment editor with DPAPI encrypted secrets, CPU, memory and
  listening ports, event history.
- MSI and NSIS installers with silent install for Intune and SCCM.

## Docs

- [Getting started](docs/getting-started.md)
- [Deployment](docs/deployment.md): installers, silent install, upgrades, access, signing
- [Configuration and environments](docs/configuration.md)
- [CLI](docs/cli.md)
- [Agent API](docs/api.md)
- [Security model](docs/security.md)
- [Troubleshooting](docs/troubleshooting.md)
- [Product plan](PLAN.md)

## Repository

```
crates/core          Settings (dev / prod), API types, detector, token
crates/agent         keeprun-agent: Windows service, supervisor, SQLite, HTTP + WebSocket API
crates/cli           keeprun: command line client
apps/desktop         Tauri 2 + Svelte 5 + Tailwind desktop app and installer config
config/              Agent settings per environment
scripts/             dev.ps1, build.ps1, text-lint.mjs
docs/                Documentation
tests/fixtures/      Test apps (process tree, crash loop)
```

## Develop

Requirements: Windows 10+, Rust stable (MSVC), Node.js 20+, WebView2.

```powershell
npm run setup        # UI dependencies
npm run dev          # dev agent on 127.0.0.1:7181 + desktop app with hot reload
npm run dev:cli -- list
npm test             # Rust tests
npm run lint         # clippy, rustfmt, text lint, svelte-check
npm run build        # release binaries + MSI and NSIS installers (prod)
```

Dev and prod are fully separate (port, data folder, service name, config), see
[configuration.md](docs/configuration.md).

## License

Apache 2.0
