# Troubleshooting

**"Cannot reach the Keeprun service at http://127.0.0.1:7171."**
Start **Keeprun Agent** in `services.msc`, or run `net start KeeprunAgent` as Administrator.
If it stops right away, read `C:\ProgramData\Keeprun\agent.log`.

**"Cannot read the agent token at C:\ProgramData\Keeprun\agent.token."**
Your account is not allowed to use Keeprun. An Administrator runs
`net localgroup "Keeprun Users" <you> /add`. Sign out and in again.

**"Port 3000 is in use by node.exe (PID 4120)."**
Another process holds the port set in `PORT`. Stop it (`taskkill /PID 4120`) or change `PORT`
in the Environment tab.

**"Another copy is running outside Keeprun."**
Something else started the same app, for example `npm run dev` in a terminal or a coding
agent. Keeprun checks every 10 seconds for a process outside Keeprun that listens on a port and
runs a file from the app folder. Once Keeprun has seen the app run, only processes that run
the app's own command or hold its port count, so dev tools such as Prisma Studio or Storybook
started from the folder are left alone. Click **End it** on the Overview tab, or start the app in
Keeprun, which ends the copy first. **Tell coding agents** adds a note to the app's
`CLAUDE.md` (or `AGENTS.md`) so agents use `keeprun logs` and `keeprun restart` instead of
starting their own copy.

**An app works in my terminal but not in Keeprun.**
The service runs as LocalSystem with the machine PATH. Use full paths in the command (the
detector does this), and set variables your terminal profile normally sets in the
Environment tab. `keeprun detect <folder>` shows the paths Keeprun would use.

**"node_modules is missing."**
Run `npm install` (or `pnpm install`, `yarn`) in the app folder once. Keeprun does not install
dependencies.

**An app is Failed.**
It crashed more times in a row than "Max restarts" allows. Read the Logs tab, fix the cause,
then click Start. To retry forever, set Max restarts to 0.

**A secret shows an error after moving the database.**
DPAPI secrets can only be decrypted by the account (and machine) that encrypted them. Enter
the value again in the Environment tab.

**Run the agent in a terminal to debug.**
```powershell
net stop KeeprunAgent
keeprun-agent run --env prod     # Administrator terminal. Ctrl+C stops it and its apps.
net start KeeprunAgent
```
