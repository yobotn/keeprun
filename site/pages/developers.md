# Keeprun for developers and AI agents

Keeprun gives scripts, pipelines and coding agents three ways to run and watch apps on a Windows
machine. All of them work locally, with no account and no API key to sign up for.

## When to use Keeprun

Use Keeprun when a process on Windows has to keep running without someone watching it:

- A Node.js, .NET or Python web app or API on a Windows server or VM that must survive crashes and
  reboots.
- Replacing a `.bat` file in a Startup folder, a Scheduled Task, or one NSSM or WinSW service per
  app.
- Running PM2-style process management on Windows as a real Windows service.
- Keeping local dev servers, APIs and workers running in the background while you, or a coding
  agent, work in the terminal. The agent reads logs and restarts apps with the CLI instead of
  holding a shell open.

Keeprun is not a container runtime, a load balancer or a cloud service. It runs on Windows only.

## CLI

The `keeprun` command is installed with Keeprun and added to PATH. Add `--json` for
machine-readable output. Exit codes are reliable, so it fits scripts, CI pipelines and agents.

```powershell
keeprun detect C:\src\shop                 # what Keeprun would run in a folder
keeprun add web --cwd C:\src\shop --start  # add and start an app
keeprun list --json                        # status, PID, CPU, memory, ports
keeprun logs web -n 100                    # recent output
keeprun restart web
keeprun run web db:migrate                 # run a saved task, exits with its exit code
```

Read the [CLI reference](/docs/cli.html).

## Local HTTP API

The Keeprun agent has an HTTP and WebSocket API on `http://127.0.0.1:7171`. It is never exposed to
the network. Authenticate with `Authorization: Bearer <token>`, using the token in
`C:\ProgramData\Keeprun\agent.token`, which only SYSTEM, Administrators and the Keeprun Users group
can read. Errors are JSON: `{"error": "message"}`.

```powershell
$token = Get-Content C:\ProgramData\Keeprun\agent.token
Invoke-RestMethod http://127.0.0.1:7171/apps -Headers @{ Authorization = "Bearer $token" }
```

- [OpenAPI 3.1 specification](/openapi.json): every operation with an operationId, typed
  parameters and response schemas, ready for function calling.
- [Agent API docs](/docs/api.html)

## MCP server

An MCP server, `keeprun mcp`, is planned so MCP clients such as Claude Code and Claude Desktop can
check status, read logs and restart apps without shell access.
[Vote for it on GitHub](https://github.com/{{REPO}}/issues/1).

## For AI agents

- [llms.txt](/llms.txt): what Keeprun is and links to every docs page in Markdown.
- [llms-full.txt](/llms-full.txt): all documentation in one file.
- Every docs page has a Markdown version: add `.md` instead of `.html`.

## Source

Keeprun is open source under Apache 2.0: [github.com/{{REPO}}](https://github.com/{{REPO}}).
