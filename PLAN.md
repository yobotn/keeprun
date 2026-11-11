# Keeprun

Run and manage internal apps on Windows. Pick a folder, click Run, and it stays up.

Status: M0 to M3 built. Next: pilot with existing customers, then v1.1. Last updated 2026-11-10.

---

## 1. Product

**Problem.** Companies host internal apps (dashboards, APIs, Node and .NET tools) on Windows Server VMs and locked-down Windows desktops. Today they keep them running with NSSM, Task Scheduler, PM2 workarounds and RDP sessions. Docker Desktop often cannot run there (no nested virtualization, licensing cost). Existing tools such as Servy work but are hard to use.

**Users.**
- Primary: IT and platform teams hosting internal apps on Windows Server.
- Secondary: developers on Windows who run many local projects (`npm run dev` and similar).

**Promise.** From install to the first running app in under 60 seconds.

**Out of scope for v1.** Public deployment, Linux hosts, running a container engine.

### Competitive landscape

| Tool | What it does | Gap Keeprun fills |
|---|---|---|
| Servy | Runs any app as a Windows service, has a GUI | Complex setup, no stack detection, no proxy or deploy |
| NSSM, WinSW | Service wrappers | No UI, NSSM barely maintained |
| PM2 on Windows | Node process manager | No real service support on Windows, no UI without paid PM2 Plus |
| Runny, GCE, startall | Run npm scripts | Local dev only, no service, small projects |
| Rancher Desktop, Podman Desktop | Container engines | Need WSL2 or Hyper-V |
| Docker Offload | Cloud container engine for VDI | Containers only, paid, cloud |
| Coolify, Dokploy | Self-hosted app platforms | Linux and Docker only |

---

## 2. Writing and design rules

- No emojis in UI, docs, installer, commit messages or logs. The one exception is the root README.md, which is the marketing page.
- No em dashes. Use periods, commas or plain hyphens.
- No filler words ("seamless", "powerful", "unlock", "supercharge", "effortless", "robust").
- UI copy is short and literal: "Start", "Stop", "Restart", "Open folder", "Copy URL".
- Error messages say what happened and what to do next. Example: "Port 3000 is in use by node.exe (PID 4120). Stop it or choose another port."
- Icons come from one SVG set (Lucide). Never emoji.
- CI runs a text lint that fails on emojis or em dashes in UI strings, docs and commit messages.

---

## 3. Architecture

```
+------------------------+   localhost HTTP + WebSocket   +--------------------------+
|  Desktop UI            | <----------------------------> |  Agent                   |
|  Tauri 2 + Svelte      |   token auth, TLS later        |  Windows service         |
|  runs as normal user   |                                |  LocalSystem by default  |
+------------------------+                                +------------+-------------+
                                                                       |
                         +-------------------+------------------+------+------------+
                         |                   |                  |                   |
                    Supervisor           Log store          Detector             SQLite
                    Job Objects          rotating files     package.json,        apps, env,
                    restart policy       + live stream      .csproj, venv        events
```

### Agent (Rust)
- `windows-service`: runs as a Windows service, starts on boot.
- `tokio` + `axum`: local HTTP API and WebSocket for live logs and status.
- `windows-rs`: Job Objects, listening ports (GetExtendedTcpTable), process info.
- `rusqlite`: apps, environment, events.
- Owns every managed process. Closing the UI never stops apps.

### Process control
- Each app runs inside its own Windows Job Object with kill-on-close. Stop kills the full process tree (npm, cmd, node children). No zombies.
- Restart policy: on crash, with exponential backoff and a max retry count, then marked "Failed".
- Graceful stop: send Ctrl+C / Ctrl+Break first, wait a timeout, then terminate the job.

### Desktop UI (Tauri 2 + Svelte + Tailwind)
- Pure client of the agent API. No admin rights for daily use.
- Same API later serves remote management of several servers.

### Accounts
- Agent runs as LocalSystem by default.
- Each app can override "Run as" (a specific account) in Settings.

### Data locations
- Config and DB: `C:\ProgramData\Keeprun\`
- Logs: `C:\ProgramData\Keeprun\logs\<app>\`

---

## 4. v1 scope (MVP)

1. **Add app.** Pick a folder, detect its type, review suggested settings, save.
   - Node: npm, pnpm, yarn scripts from `package.json`, or `node file.js`.
   - .NET: `dotnet run` or a published dll / exe.
   - Python: detect venv, run a script or module (uvicorn, flask, etc).
   - Custom: any executable or command line.
2. **Run.** Start, stop, restart. Optional start on boot. Restart on crash with backoff.
3. **Logs.** Live stdout and stderr with search, pause, copy, clear. Rotation by size.
4. **Environment.** Editor with `.env` import. Secret values masked.
5. **Health.** Status, PID, CPU, RAM, uptime, listening ports, "Open in browser".
6. **Events.** Started, stopped, crashed, restarted, config changed.
7. **CLI.** `keeprun list`, `keeprun start <app>`, `keeprun logs <app> -f`.
8. **Installer.** MSI that installs the agent service and the UI. Silent install for Intune / SCCM.

---

## 5. UX

- **Layout.** Sidebar with apps (grouped by tag, status dot) plus a detail view with tabs: Overview, Logs, Environment, Settings, Events.
- **Command palette (Ctrl+K).** Jump to an app, start, stop, restart, open logs, add app.
- **Empty state.** One clear "Add your first app" action that opens the folder picker.
- **Keyboard first.** Every action has a shortcut. Log view fully keyboard driven.
- **Themes.** Light, dark, follow system.
- **Quality bar.** Linear, Raycast, GitHub Desktop.

```
+---------------------------------------------------------------+
| Keeprun            [ Add app ]             Search (Ctrl+K)  [S] |
+--------------+------------------------------------------------+
| APPS         |  hr-portal                  Running   :3001    |
| * hr-portal  |  Overview | Logs | Environment | Settings | Events
| * sales-api  |  ----------------------------------------------  |
| o reports    |  CPU 3%    RAM 142 MB    Uptime 4d 2h          |
| ! legacy-bot |  Folder   D:\apps\hr-portal     [Open folder]  |
|              |  Command  npm run start                        |
|              |  URL      http://localhost:3001  [Open]        |
|              |                                                |
|              |  [Stop]  [Restart]                             |
|              |                                                |
|              |  Recent events                                 |
|              |  10:42  Restarted after crash (exit code 1)    |
|              |  09:00  Started                                |
+--------------+------------------------------------------------+
```

---

## 6. Milestones

| Phase | Content | Estimate |
|---|---|---|
| M0 Spike | Agent as a service, starts `npm run dev` in a Job Object, streams logs to a basic UI. Pilot with existing customers. | 1 week |
| M1 Core | Detector, SQLite, start / stop / restart, restart policy, boot start, CLI | 2 weeks |
| M2 UI | Full layout, tabs, log view, env editor, command palette, themes | 2 weeks |
| M3 Ship | MSI, code signing, auto-update, docs site, public beta | 1 week |
| v1.1 | Reverse proxy, internal hostnames, HTTPS with company certificate | 2 to 3 weeks |
| v1.2 | Deploy from Git, zip or network share, rollback | 2 weeks |
| v2 | Multi-server, AD / Entra ID login, roles, audit log, Docker connector (remote, Podman, WSL, Offload) | later |

### M0 task list

1. Cargo workspace: `crates/agent`, `crates/core`, `apps/desktop`, `crates/cli`.
2. Agent runs in console mode (dev) and as a Windows service (install / uninstall commands).
3. Spawn a command in a Job Object with kill-on-close, capture stdout and stderr.
4. Graceful stop (Ctrl+Break), then terminate the job after a timeout.
5. Axum API: `GET /apps`, `POST /apps/{id}/start`, `POST /apps/{id}/stop`, WebSocket `/apps/{id}/logs`.
6. Token auth for the local API (token file readable by Administrators and the UI user).
7. Minimal Tauri + Svelte UI: list apps, start / stop, live log panel.
8. Test: `npm run dev` on a Vite project, stop it, confirm no leftover node.exe.
9. CI: build, test, text lint (no emojis, no em dashes).

---

## 7. Open source and business model

**License.** Apache 2.0. Fully open source. No closed or paid-only features in the code.

**Revenue comes from support.**

| Offer | Includes |
|---|---|
| Support plan (yearly, per company) | SLA response times, private ticket channel, upgrade help |
| Enterprise plan | Support plan plus signed LTS builds, early security advisories, named contact |
| Onboarding / migration | Fixed price. Move apps off NSSM, Task Scheduler or scripts. |
| Sponsored features | Customer funds a feature, it ships to everyone |

Enterprises pay for accountability, not for code. A clone of the repo does not include the support relationship.

**First customers.** Existing customer base acts as pilot users from M0.

---

## 8. Risks

| Risk | Mitigation |
|---|---|
| Antivirus flags an unsigned agent running as SYSTEM | Code signing from the first release (Azure Trusted Signing) |
| LocalSystem cannot find user-installed Node or Python | Detector stores full absolute paths. Per-app "Run as" override. |
| npm on Windows spawns `cmd` chains that are hard to stop | Job Objects with kill-on-close. Covered by M0 test. |
| Rust slows development | Tight v1 scope. UI logic in Svelte, Rust side small and focused. |
| Servy improves its UI | Compete on onboarding speed, detection, proxy and deploy |
| Local API abuse by other users on the machine | Token auth, bind to 127.0.0.1 only, token file ACL |

---

## 9. Brand

- Name: **Keeprun**
- CLI: `keeprun`
- Domain: `keeprun.app` (website and update manifest). Repository: `github.com/yobotn/keeprun`.
- To verify: trademark search (USPTO / EUIPO).

## 10. Decisions made during the build

| Decision | Why |
|---|---|
| Detector runs in the UI and CLI (user context), not in the agent | LocalSystem cannot see per-user tool installs. Detected commands use absolute paths, and the agent puts that tool folder first on PATH at start, so `npm.cmd` finds `node.exe`. |
| Desired state works like Docker "unless-stopped" | Apps a user started come back after a reboot or agent crash. Apps a user stopped stay stopped. Agent shutdown does not change desired state. |
| Restart policy: on-failure by default, backoff 1s to 60s, Failed after 5 in a row, reset after 60s stable | A crash loop must not burn CPU or flood logs, and a long-running app that crashes once a week must never reach Failed. |
| Failed apps get one more bounded retry cycle when the agent starts | After a reboot the dependency that caused the crash (database, network share) is often back. |
| Ports come from the OS (GetExtendedTcpTable over the job's processes) | "Open in browser" must be right even when the port is set in code, not in `PORT`. |
| Port check before start | Gives the plan's error ("Port 3000 is in use by node.exe (PID 4120)") instead of an app crash loop. |
| Secret env values encrypted with DPAPI | The database sits in ProgramData. Clear-text passwords there fail most security reviews. |
| "Keeprun Users" local group for token access | The UI runs non-elevated, where UAC hides Administrators membership. A group is manageable by IT with `net localgroup`. |
| Agent service has SCM recovery actions | If the agent crashes, Windows restarts it, and it restarts the apps. |
| `keeprun-agent install` is idempotent and adds the CLI to PATH | Installers call it on every install and upgrade. PATH is edited in Rust, not NSIS, to avoid truncation. |
| Desktop binary named `keeprun-desktop.exe` | `Keeprun.exe` and the CLI `keeprun.exe` would collide on a case-insensitive file system. |
| Deferred: per-app "Run as" | Needs stored passwords or gMSA and `CreateProcessAsUser`. Own design, v1.1. |
| Auto-update in the desktop app, user approved | The app checks `keeprun.app/latest.json`, shows an Update button, and runs the signed MSI or setup.exe after UAC approval. The service never updates itself. IT can turn it off or use a mirror with `HKLM\SOFTWARE\Policies\Keeprun` (`DisableUpdates`, `UpdateUrl`), also settable at install. Publishing a GitHub release is the rollout switch. |
| Website on GitHub Pages | `site/` plus `docs/` rendered to HTML, deployed by Actions. Download links and the update manifest come from the latest published release, so the site has no version to edit. |
| App port is the PORT variable | One source of truth. A Port field edits it, detected Python commands read `%PORT%`, ASP.NET Core gets `ASPNETCORE_URLS`, and two apps cannot share a port. |
| Code signing wired, not configured | `KEEPRUN_SIGN_COMMAND` feeds Tauri's `signCommand`. Needs the certificate (open item 3). |
| Tasks separate from the run command | Detected one-off scripts (install, build, migrate, seed) were offered as ways to run the app. They are now saved as tasks and only run on demand, from the Tasks tab or `keeprun run`. One task at a time per app, never restarted. |
| Port changed with `keeprun-agent set-port` | One command checks the port is free, writes `config.toml` and restarts the service. UI and CLI read the same file. Installers take `KEEPRUN_PORT` (MSI) or `/PORT=` (NSIS). |
| MSI Repair enabled | The Tauri template sets `ARPNOREPAIR`. Repair is the normal fix for a broken service registration, so the fragment clears it. |
| Agent drops `NoDefaultCurrentDirectoryInExePath` for apps | Hardened servers set it, which stops `start.bat` in the app folder from being found. |

## 11. Open items

1. Trademark search for Keeprun. Domain (`keeprun.app`) and repository (`yobotn/keeprun`) are set. docs/releasing.md covers changing the domain.
2. Support plan pricing and SLA times.
3. Code signing provider and certificate.
