# Agent API

Local HTTP and WebSocket API on `127.0.0.1:7171` (prod) or `:7181` (dev). Every route except
`/health` needs the token from `<data>\agent.token`:

- HTTP: `Authorization: Bearer <token>`
- WebSocket: `?token=<token>` (browsers cannot set headers on WebSockets)

Errors return `{"error": "..."}` with 400, 401, 404, 409 or 500.

| Method | Path | Body or query | Returns |
|---|---|---|---|
| GET | `/health` | | `{ok, env, version}` |
| GET | `/apps` | | `AppView[]` |
| POST | `/apps` | `NewApp` | `AppView`, 201 |
| GET | `/apps/{id}` | | `AppView` |
| PATCH | `/apps/{id}` | any `NewApp` fields | `AppView` |
| DELETE | `/apps/{id}` | | 204. App must be stopped. |
| POST | `/apps/{id}/start` | | `AppView`. 409 if running, 400 with the reason if it cannot start (port in use, missing folder). |
| POST | `/apps/{id}/stop` | | `AppView` |
| POST | `/apps/{id}/restart` | | `AppView` |
| POST | `/apps/{id}/tasks/run` | `{name}` (saved task) or `{command}` | `AppView` with `task`. Returns once started. 409 if a task is running. |
| POST | `/apps/{id}/tasks/stop` | | `AppView`. Ends the task's process tree. |
| GET | `/apps/{id}/logs/recent` | `?tail=200` | `LogLine[]` |
| WS | `/apps/{id}/logs` | `?token=` | Buffered lines, then live `LogLine` JSON messages |
| GET | `/apps/{id}/events` | `?limit=100` | `Event[]`, newest first |
| GET | `/events` | `?limit=100` | `Event[]` for all apps |

## Types

```ts
NewApp = {
  name: string; cwd: string; command: string;
  env?: Record<string, string>; secrets?: string[]; tags?: string[];
  autostart?: boolean;                               // default true
  restart?: "never" | "on-failure" | "always";       // default "on-failure"
  max_restarts?: number;                             // default 5, 0 = no limit
}
AppView = NewApp & {
  id: string;
  state: "stopped" | "starting" | "running" | "stopping" | "exited" | "restarting" | "failed";
  pid: number | null; exit_code: number | null; started_at: number | null;   // ms
  restarts: number; next_restart_at: number | null;
  stats: { cpu_percent: number; memory_bytes: number; processes: number; ports: number[] } | null;
}
LogLine = { ts: number; stream: "stdout" | "stderr" | "system"; text: string }
Event = { id: number; app_id: string; ts: number; kind: string; message: string }
```

Event kinds: `added`, `config-changed`, `started`, `start-failed`, `stopped`, `exited`,
`crashed`, `restarted`, `failed`.
