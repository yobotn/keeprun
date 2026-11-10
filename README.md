<div align="center">

<img src="apps/desktop/app-icon.svg" width="96" alt="Keeprun logo" />

# Keeprun

### 🚀 Pick a folder. Click Run. It stays up.

**Run your internal Node, .NET and Python apps on Windows as real services, with no Docker, no WSL and no Hyper-V.**

[🌐 keeprun.app](https://keeprun.app) · [⚡ Get started](docs/getting-started.md) · [📦 Deploy at scale](docs/deployment.md) · [⌨️ CLI](docs/cli.md)

![Windows](https://img.shields.io/badge/Windows-10%2B%20%7C%20Server-0078D6?logo=windows&logoColor=white)
![Rust](https://img.shields.io/badge/Rust-agent-B7410E?logo=rust&logoColor=white)
![Tauri](https://img.shields.io/badge/Tauri%202-desktop-24C8DB?logo=tauri&logoColor=white)
![License](https://img.shields.io/badge/license-Apache%202.0-green)

</div>

---

## 😩 The problem

Your company has a dozen internal dashboards, APIs and tools running on Windows Server VMs.
Today they stay alive thanks to some mix of **NSSM**, **Task Scheduler**, **PM2 hacks** and
someone RDP-ing in at 2am to restart `node.exe`.

Docker Desktop? There's no nested virtualization, it needs a license, and IT says no.

## ✨ The fix

Keeprun turns any app folder into a supervised, auto-restarting Windows service **in under 60
seconds**. It comes with a desktop app your team will actually enjoy and a CLI for scripts.

```text
📁 Pick folder  →  🔍 Stack detected  →  ▶️ Save and start  →  ✅ Running. Forever.
```

---

## 🔥 Features

| | |
|---|---|
| 🔍 **Smart stack detection** | Finds Node (npm, pnpm, yarn, bun), .NET, Python (venv, uvicorn, Flask, Django) and custom `.exe`, `.bat` and `.ps1` apps, then suggests the right command with full paths that work under LocalSystem. |
| 🛡️ **No zombie processes** | Every app runs in its own Windows Job Object. Stop kills the **whole process tree**, so nothing is left behind holding your port. |
| ♻️ **Self-healing** | Crashed apps restart with exponential backoff, come back after a reboot, and get marked **Failed** if they keep crashing, so a broken app can't hammer your server. |
| 📜 **Live logs** | Stream logs in real time with search. No more digging through `C:\whatever\out.log`. |
| 🔐 **Encrypted secrets** | Environment editor with DPAPI encryption. Import a `.env` and keys like `PASSWORD`, `TOKEN` and `API_KEY` get encrypted automatically. |
| 📊 **Know what's running** | CPU, memory, listening ports and a full event history: started, crashed, restarted, changed. |
| 🧰 **Saved tasks** | One-click migrations, seeds and builds that run in the app folder with the app's environment. |
| 🏢 **Enterprise-ready installers** | MSI and NSIS with silent install for **Intune** and **SCCM**. |
| ⌨️ **CLI included** | Everything the UI can do, scriptable, with `--json` output. |

---

## ⚡ 60-second quickstart

1. 📥 Install `Keeprun_<version>_x64-setup.exe` (or the `.msi`).
2. 🖱️ Open Keeprun and hit **Add app** (`Ctrl+N`).
3. 📁 Pick your app folder. Keeprun figures out the rest.
4. ▶️ Click **Save and start**.

That's it. 🎉 Reboot the server and it comes back on its own.

### Prefer the terminal? 💻

```powershell
keeprun add my-api --cwd C:\apps\my-api --start   # detect, add and start
keeprun list                                      # status, PID, CPU, memory, ports
keeprun logs my-api -f                            # follow logs live
keeprun run my-api migrate                        # run a saved task
```

---

## 🤔 Why not...?

| | Keeprun | NSSM / WinSW | PM2 on Windows | Docker Desktop |
|---|:---:|:---:|:---:|:---:|
| Real Windows service | ✅ | ✅ | ⚠️ | ✅ |
| Desktop UI | ✅ | ❌ | 💰 paid | ✅ |
| Stack detection | ✅ | ❌ | ❌ | ❌ |
| Kills the whole process tree | ✅ | ⚠️ | ⚠️ | ✅ |
| Encrypted secrets | ✅ | ❌ | ❌ | ⚠️ |
| No WSL or Hyper-V needed | ✅ | ✅ | ✅ | ❌ |
| Silent install for Intune and SCCM | ✅ | ⚠️ | ❌ | ⚠️ |

---

## 📚 Docs

- 🏁 [Getting started](docs/getting-started.md)
- 📦 [Deployment](docs/deployment.md): installers, silent install, upgrades, access, signing
- ⚙️ [Configuration and environments](docs/configuration.md)
- ⌨️ [CLI](docs/cli.md)
- 🔌 [Agent API](docs/api.md)
- 🔒 [Security model](docs/security.md)
- 🩺 [Troubleshooting](docs/troubleshooting.md)
- 🗺️ [Product plan](PLAN.md)

---

## 🛠️ Contributing

### 🏗️ How it's built

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

### 🧑‍💻 Develop

You need Windows 10+, Rust stable (MSVC), Node.js 20+ and WebView2.

```powershell
npm run setup        # UI dependencies
npm run dev          # dev agent on 127.0.0.1:7181 + desktop app with hot reload
npm run dev:cli -- list
npm test             # Rust tests
npm run lint         # clippy, rustfmt, text lint, svelte-check
npm run build        # release binaries + MSI and NSIS installers (prod)
```

Dev and prod are fully separate (port, data folder, service name, config). See
[configuration.md](docs/configuration.md).

---

<div align="center">

**📄 Apache 2.0** · Made for everyone who's tired of RDP-ing in to restart `node.exe` 💙

⭐ **Star the repo if Keeprun saved your weekend!** ⭐

</div>
