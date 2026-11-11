# About Keeprun

Keeprun is an open source tool that keeps internal apps running on Windows. You point it at a
folder, it works out how to run the app, and one Windows service keeps it running: it restarts
the app when it crashes, starts it again after a reboot, and keeps its logs and history in one
place. It runs Node.js, .NET and Python apps, and any other command line.

## Why it exists

Many companies host their internal web apps, APIs and tools on Windows Server VMs and locked-down
Windows desktops. Those apps are often kept alive with a `.bat` file in a Startup folder, a
Scheduled Task, or a service wrapper that someone set up years ago. Crashes go unnoticed, logs sit
in a console window nobody sees, and one person knows how it all fits together.

Containers would solve part of this, but Docker Desktop needs a license for larger companies,
WSL or Hyper-V, and nested virtualization that many Windows VMs do not have. Keeprun gives the
same everyday benefits, restart on crash, start on boot, logs and health in one place, while
your app runs as a normal Windows process.

## Who it is for

- **IT and operations teams** that run internal apps on Windows servers and want one way to run,
  watch and deploy them, including silent installs with Intune, SCCM or Group Policy.
- **Developers** who build those apps, and who want their own dev servers to keep running in the
  background while they work.

## How it is built

Keeprun is written in Rust. The agent is a Windows service with a local API on 127.0.0.1, each app
runs in its own Windows Job Object, and settings are stored in SQLite. The desktop app is built
with Tauri and Svelte, and the `keeprun` command line talks to the same API. There is no cloud
service, no account and no telemetry. Read the [security model](/docs/security.html).

## Open source

Keeprun is free for personal and commercial use under the
[Apache 2.0 license](https://github.com/{{REPO}}/blob/main/LICENSE). The source code, releases,
issues and the [roadmap](/docs/roadmap.html) are public on
[GitHub](https://github.com/{{REPO}}). Contributions and feature votes are welcome.
