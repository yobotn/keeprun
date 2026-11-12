# MCP server

`keeprun mcp` runs a [Model Context Protocol](https://modelcontextprotocol.io) server over stdio.
Coding agents and MCP clients such as Claude Code, Claude Desktop, Cursor and VS Code check app
status, read logs and errors, and restart apps with typed tools instead of a shell.

It talks to the local agent with your token, like the CLI, so there is no new port or service.
Environment variable values are never returned, only their names.

## Add it to a client

Claude Code, for every project:

```powershell
claude mcp add --scope user keeprun -- keeprun mcp
```

Other clients take the same command in their MCP settings:

```json
{
  "mcpServers": {
    "keeprun": { "command": "keeprun", "args": ["mcp"] }
  }
}
```

The client starts the server in the project folder. Tools that take an app default to the app
whose folder holds that folder, so an agent working in `D:\code\shop` reaches the `shop` app
without naming it.

## Tools

| Tool | What it does |
|---|---|
| `list_apps` | Every app with state, URL, folder, command and current errors |
| `app_status` | State, URL, ports, CPU, memory, restarts, errors, health check, env variable names, tasks, and copies running outside Keeprun |
| `read_logs` | Recent output, with `tail`, `errors_only` and `search` |
| `get_errors` | Errors the app printed while running, as a ready fix prompt |
| `list_events` | Started, stopped, crashed, restarted, config changed |
| `start_app` | Start, end copies running outside Keeprun, and wait until it runs |
| `stop_app` | Stop the app and its process tree |
| `restart_app` | Restart and wait until it runs. Returns the first output of the new run |
| `run_task` | Run a saved task and return its output. Only with `--allow-tasks` |

## Options

```text
keeprun mcp                  Read tools plus start, stop and restart
keeprun mcp --read-only      Only status, logs, errors and events
keeprun mcp --allow-tasks    Also run saved tasks (build, migrate, test)
keeprun mcp --env dev        Use the dev agent
```

Adding or removing apps, changing variables and running arbitrary commands are not available
through MCP. Use the desktop app or the CLI for those.

## Why use it

An agent that starts `npm run dev` itself starts a second copy of an app Keeprun already runs.
The copy fights over the port, and its errors never reach Keeprun. With the MCP server the agent
reads the logs and restarts the app Keeprun runs. The server tells clients this when they
connect.
