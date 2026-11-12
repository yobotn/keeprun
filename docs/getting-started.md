# Getting started

## Install

Run `Keeprun_<version>_x64-setup.exe` or `Keeprun_<version>_x64_en-US.msi`. The installer:

- installs the desktop app, the `keeprun-agent` service and the `keeprun` CLI,
- adds **Open with Keeprun** to the Explorer menu for folders,
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
3. Choose a command, and the env file to use: `.env`, `.env.local`, `.env.production` or any
   other `.env.*` file in the folder, or none. Keeprun reads the file every time the app starts.
   When you edit it while the app runs, the Overview tab lists the changed keys and offers a
   restart. Variables you add in Keeprun override the file. Keys that look like secrets
   (`PASSWORD`, `TOKEN`, `API_KEY` and so on) are encrypted.
4. Click **Save and start**. The Logs tab opens.

You can also right-click a folder in Explorer, or the empty space inside one, and choose
**Open with Keeprun** (on Windows 11 it is under **Show more options**). If the folder is already
an app, Keeprun starts it and shows its logs. If not, the Add app dialog opens for that folder so
you can pick what to run.

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
| You click Disable | The app is stopped and moved to the **Disabled** section. It cannot be started and does not start on boot until you click **Enable**. |

## The window

- The sidebar lists apps in three sections: **Running**, **Off** and **Disabled** (shown when an
  app is disabled). Click a section heading to collapse it. Keeprun reopens the app and tab you
  had open last.
- Keeprun opens maximized. Closing the window keeps it in the notification area, where its icon
  shows app health: green when all is well, yellow while apps start or stop, orange while an app
  crashes and restarts, red when an app failed or the agent cannot be reached. No dot means
  nothing is running. Turn this off in **Preferences** (the gear button) to quit on close.
- File paths in logs and in error output, such as `src\routes\users.ts:42:17` in a stack trace,
  are links. Click one to open the file at that line in VS Code, Cursor or Windsurf, whichever is
  installed. Paths relative to the app folder work too. Choose the editor in **Preferences**.
- A **health check** (Settings) catches an app that runs but no longer answers, such as a
  server stuck on a lock. Enter a path such as `/health`: Keeprun requests it on the app's port
  every 10 seconds, and any 2xx or 3xx answer within 5 seconds passes. The Overview tab shows the
  result. Turn on the restart option to restart the app after 3 failed checks in a row. If the
  check still fails after that restart, Keeprun stops restarting and says so, since the check
  itself is probably wrong.
- When the app folder is in a Git repository, the Overview tab shows the branch, commits to pull
  or push, uncommitted changes and the last commit. **Pull and restart** fast-forwards the
  branch (it never merges), runs the app's `install` task when a lockfile or other dependency
  list changed, then restarts the app. Git runs as you, with your credentials.
- When something else runs a copy of the app, such as `npm run dev` started by a coding agent,
  the Overview tab says so and offers **End it**. **Tell coding agents** adds a note to the
  app's `CLAUDE.md` or `AGENTS.md` so they use Keeprun instead, or connect them with the
  [MCP server](mcp.md).
- **Preferences** also sets the language and theme. The UI is available in Arabic, Chinese
  (Simplified), Dutch, English, French, German, Hebrew, Italian, Japanese, Korean, Polish,
  Portuguese (Brazil), Russian, Spanish, Turkish and Ukrainian, and follows the Windows language by
  default. Arabic and Hebrew use a right-to-left layout. Messages from the agent, such as events
  and errors, stay in English.

## Keyboard

| Keys | Action |
|---|---|
| Ctrl+K | Command palette: jump to an app, start, stop, disable, open logs, change theme or language |
| Ctrl+N | Add app |
| Ctrl+Enter | Start or stop the selected app |
| Ctrl+Shift+R | Restart the selected app |
| Ctrl+1 to Ctrl+6 | Overview, Logs, Tasks, Environment, Settings, Events |
| Alt+Up, Alt+Down | Previous or next app |
| / or Ctrl+F | Search logs |
| P, E, Shift+G | Logs: pause, errors only, jump to latest |
