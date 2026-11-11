# CLI

`keeprun` talks to the local agent. Add `--env dev` to use the dev agent and `--json` for
machine-readable output.

```text
keeprun status                         Agent, environment and data folder
keeprun list                           Apps with status, PID, CPU, memory and ports
keeprun detect [folder]                What Keeprun would run in a folder

keeprun add [name] [--cwd <folder>]    Add an app. Without a command, the best detected one.
    [-e KEY=VALUE]... [--env-file f]   Variables. A .env in the folder is imported by default.
    [-t tag]... [--restart never|on-failure|always] [--no-autostart] [--start]
    [--port n]                         Sets PORT. Defaults to the detected port.
    [-- <command line>]

keeprun set <app> [--name] [--cwd] [--command] [--restart] [--max-restarts n]
                  [--autostart true|false] [--tags a,b] [--port n, 0 removes it]
keeprun env <app>                      Show variables (secrets masked, --reveal to show)
keeprun env <app> -e K=V [--secret]    Set variables
keeprun env <app> --unset K            Remove a variable
keeprun env <app> --import .env        Import a file

keeprun start|stop|restart <app>
keeprun tasks <app>                    Saved tasks and the last run
keeprun tasks <app> --add <name> -- <command>
keeprun tasks <app> --rm <name>
keeprun run <app> <task>               Run a saved task. Shows output, exits with its exit code.
keeprun run <app> -- <command line>    Run a command once in the app folder with the app env
keeprun logs <app> [-n 200] [-f]       Recent lines, -f to follow
keeprun events [app] [-n 30]           Started, stopped, crashed, restarted, config changed
keeprun rm <app>                       Remove a stopped app
```

Examples:

```powershell
keeprun add --cwd D:\apps\hr-portal --tag finance --start
keeprun add reports --cwd D:\apps\reports -e PORT=8080 -- python -m uvicorn main:app --port 8080
keeprun env hr-portal -e DB_PASSWORD=... --secret
keeprun logs hr-portal -f
keeprun run hr-portal db:migrate
keeprun list --json | ConvertFrom-Json | Where-Object state -eq failed
```

## Tasks

Tasks are one-off commands such as migrations, seeds or installs. `keeprun add` and the desktop
app save the ones they detect (for example `db:migrate` and `seed` from `package.json`,
`manage.py migrate` for Django) but never run them on start. A task runs in the app folder with
the app's environment, secrets included, under the agent account. It does not start or stop the
app. One task runs at a time per app. Ctrl+C during `keeprun run` stops the task.

Task output also appears in `keeprun logs`, marked as task output.

Exit code is 1 on any error, with the message on stderr. `keeprun run` exits with the task's
exit code.
