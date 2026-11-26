# Keeprun CLI

`keeprun` talks to the local agent. Add `--env dev` to use the dev agent and `--json` for
machine-readable output.

`kr` is the short name: installed Keeprun puts `kr.exe` next to `keeprun.exe`. In an app's
folder, or a folder inside it, commands that take an app need no app name, and words such as
"the app" are skipped, so `kr restart`, `kr restart the app` and `keeprun restart hr-portal` do
the same thing from `D:\apps\hr-portal`.

```text
keeprun status [app]                   The app here: state, URL, errors, and whether its
                                       database needs migrating. Elsewhere: agent,
                                       environment and data folder
keeprun list                           Apps with status, PID, CPU, memory and ports
keeprun detect [folder]                What Keeprun would run in a folder

keeprun add [name] [--cwd <folder>]    Add an app. Without a command, the best detected one.
    [-e KEY=VALUE]... [--env-file f]   Variables, and an env file read on every start. Without
    [--no-env-file]                    --env-file the folder's .env is linked, else its first
                                       .env.* file that is not a template like .env.example.
    [-t tag]... [--restart never|on-failure|always] [--no-autostart] [--start]
    [--port n]                         Sets PORT. Defaults to the detected port.
    [-- <command line>]

keeprun set <app> [--name] [--cwd] [--command] [--restart] [--max-restarts n] [--watch-errors false]
                  [--autostart true|false] [--tags a,b] [--port n, 0 removes it]
                  [--env-file .env.local, none unlinks it]
                  [--health /health, none removes it] [--health-restart true|false]
                  [--before-start <task>, none turns it off]
keeprun env <app>                      Show variables (secrets masked, --reveal to show), the
                                       env file keys, and keys changed since the app started
keeprun env <app> -e K=V [--secret]    Set variables
keeprun env <app> --unset K            Remove a variable
keeprun env <app> --import .env        Copy a file's values into Keeprun

keeprun start|restart <app> [--no-wait]  Waits until the app runs, then prints its URL, or the
                                       errors and output of a run that did not stay up
                                       (exit code 1)
keeprun stop <app>
keeprun migrate <app> [--no-restart]   Run the app's migrate task, then restart the app, or
                                       start it when it failed on the missing migration
keeprun migrate <app> --dismiss        The database is up to date: clear the notice
keeprun tasks <app>                    Saved tasks and the last run
keeprun tasks <app> --add <name> -- <command>
keeprun tasks <app> --rm <name>
keeprun run <app> <task>               Run a saved task. Shows output, exits with its exit code.
keeprun run <app> -- <command line>    Run a command once in the app folder with the app env
keeprun logs <app> [-n 200] [-f]       Recent lines, -f to follow
keeprun pull <app> [--no-restart]      Fast-forward its Git branch, run its install task when
    [--no-migrate]                     dependencies changed and its migrate task when
                                       migrations changed, then restart it. Never merges.
keeprun issue <app>                    Errors the app printed while running, as a prompt for a coding agent
keeprun issue <app> --clear            Dismiss them
keeprun events [app] [-n 30]           Started, stopped, crashed, restarted, config changed
keeprun mcp [--read-only] [--allow-tasks]  MCP server for coding agents, see mcp.md
keeprun agent-setup <app> [--no-mcp]   Tell coding agents to use Keeprun: a note in the app's
                                       CLAUDE.md or AGENTS.md, and the MCP server in Claude
                                       Code when it is installed
keeprun rm <app>                       Remove a stopped app
```

Examples:

```powershell
keeprun add --cwd D:\apps\hr-portal --tag finance --start
keeprun add reports --cwd D:\apps\reports -e PORT=8080 -- python -m uvicorn main:app --port 8080
keeprun env hr-portal -e DB_PASSWORD=... --secret
keeprun logs hr-portal -f
keeprun run hr-portal db:migrate
kr restart                                  # the app in the current folder
kr migrate                                  # migrate, then restart
keeprun list --json | ConvertFrom-Json | Where-Object state -eq failed
```

## Tasks

Tasks are one-off commands such as migrations, seeds or installs. `keeprun add` and the desktop
app save the ones they detect (for example `db:migrate` and `seed` from `package.json`,
`manage.py migrate` for Django) but never run them on start. A task runs in the app folder with
the app's environment, secrets included, under the agent account. It does not start or stop the
app. One task runs at a time per app. Ctrl+C during `keeprun run` stops the task.

Task output also appears in `keeprun logs`, marked as task output.

## Migrations

Keeprun never migrates on its own: an app that crashes on start would run its migrations again
on every restart. It tells you when a migration is needed, and runs it when you ask.

- **When one is needed.** The Overview tab, `keeprun status` and the MCP server's `app_status`
  show it when the app prints an error that means the schema is behind, such as Django's
  "You have 3 unapplied migration(s)", `relation "users" does not exist` from PostgreSQL, Prisma's
  "does not exist in the current database" or SQLite's `no such table`. They also show migration
  files added since the last successful migrate run, for example after a pull. The first check
  of an app records the files it has. A migration applied from a terminal still shows: dismiss
  it with `keeprun migrate <app> --dismiss` or **Dismiss**.
- **The migrate task** is the saved task named `migrate`, or else the first one whose name or
  command applies migrations: `prisma migrate deploy`, `manage.py migrate`, `alembic upgrade`,
  `dotnet ef database update`, `knex migrate:latest` and similar. A task that creates, resets or
  rolls back migrations (`prisma migrate dev`, `makemigrations`, `migrate:rollback`) is never
  picked. `keeprun tasks <app>` names the one it uses.
- **Migrate and restart.** `keeprun migrate <app>`, **Migrate and restart** on the Overview tab
  and the MCP tool `migrate_app` run that task with its output, then restart the app. When the
  task fails, the app is left as it is.
- **On pull.** `keeprun pull` and **Pull and restart** run the migrate task before the restart
  when the pull changed a migration.
- **Before every start.** For a dev app whose database should always be current,
  `keeprun set <app> --before-start migrate` (or **Task before start** in Settings) runs the
  task before each start, restart and agent start. The app starts only when it succeeds. It does
  not run when Keeprun restarts the app after a crash.

Exit code is 1 on any error, with the message on stderr. `keeprun run` exits with the task's
exit code.
