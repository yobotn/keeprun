# Getting started

## Install

Run `Keeprun_<version>_x64-setup.exe` or `Keeprun_<version>_x64_en-US.msi`. The installer:

- installs the desktop app, the `keeprun-agent` service and the `keeprun` CLI,
- registers the **Keeprun Agent** service (starts on boot, restarts itself if it crashes),
- gives the installing user access to the agent, and adds the CLI to PATH.

For silent installs, see [deployment.md](deployment.md).

## Add your first app

1. Open Keeprun and click **Add app** (Ctrl+N).
2. Pick the app folder. Keeprun detects the stack:
   - **Node.js**: scripts from `package.json`, package manager from the lock file (npm, pnpm,
     yarn, bun). `start` is suggested before `dev`.
   - **.NET**: a published `.exe` or `.dll` if one exists, otherwise `dotnet run -c Release`.
   - **Python**: the `.venv` or `venv` interpreter, with uvicorn, Flask, Django or a plain
     script.
   - **Custom**: any `.exe`, `.bat`, `.cmd` or `.ps1` in the folder, or a command you type.
3. Choose a command. Variables from a `.env` file in the folder can be imported. Keys that look
   like secrets (`PASSWORD`, `TOKEN`, `API_KEY` and so on) are encrypted.
4. Click **Save and start**. The Logs tab opens.

Commands use full paths (for example `"C:\Program Files\nodejs\npm.cmd" run start`), so they
work when the service runs as LocalSystem, which does not see per-user tools on PATH.

## What "keeps running" means

| Situation | What happens |
|---|---|
| The app exits with an error | Restarted after 1s, 2s, 4s ... up to 60s. After 5 crashes in a row it is marked **Failed**. A run longer than 60s resets the count. |
| The app exits with code 0 | Stays **Exited** (policy "On failure"). Use "Always" to restart it anyway. |
| You click Stop | Ctrl+Break is sent, then the whole process tree is ended after the stop timeout. It stays stopped, also after a reboot. |
| The server reboots | The service starts and starts every app that was running, if "Start with the agent" is on. |
| The agent crashes | Windows ends every app tree (Job Objects), then restarts the service, which starts the apps again. |

## Keyboard

| Keys | Action |
|---|---|
| Ctrl+K | Command palette: jump to an app, start, stop, open logs, change theme |
| Ctrl+N | Add app |
| Ctrl+Enter | Start or stop the selected app |
| Ctrl+Shift+R | Restart the selected app |
| Ctrl+1 to Ctrl+5 | Overview, Logs, Environment, Settings, Events |
| Alt+Up, Alt+Down | Previous or next app |
| / or Ctrl+F | Search logs |
| P, E, Shift+G | Logs: pause, errors only, jump to latest |
