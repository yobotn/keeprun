// Client for the local Keeprun agent API, plus desktop-side commands.
import { invoke } from "@tauri-apps/api/core";

export type Env = "dev" | "prod";
export type AppState = "stopped" | "starting" | "running" | "stopping" | "exited" | "restarting" | "failed";
export type LogStream = "stdout" | "stderr" | "system";
export type RestartPolicy = "never" | "on-failure" | "always";
export type EventKind =
  | "added"
  | "config-changed"
  | "started"
  | "start-failed"
  | "stopped"
  | "exited"
  | "crashed"
  | "restarted"
  | "failed";

export interface Stats {
  cpu_percent: number;
  memory_bytes: number;
  processes: number;
  ports: number[];
}

export interface AppSpec {
  id: string;
  name: string;
  cwd: string;
  command: string;
  env: Record<string, string>;
  secrets: string[];
  tags: string[];
  autostart: boolean;
  restart: RestartPolicy;
  max_restarts: number;
}

export interface AppView extends AppSpec {
  state: AppState;
  pid: number | null;
  exit_code: number | null;
  started_at: number | null;
  restarts: number;
  next_restart_at: number | null;
  stats: Stats | null;
}

export type NewApp = Omit<AppSpec, "id">;
export type UpdateApp = Partial<NewApp>;

export interface LogLine {
  ts: number;
  stream: LogStream;
  text: string;
}

export interface AppEvent {
  id: number;
  app_id: string;
  ts: number;
  kind: EventKind;
  message: string;
}

export interface Suggestion {
  label: string;
  command: string;
  detail: string | null;
}

export interface Detection {
  kind: "node" | "dotnet" | "python" | "custom";
  name: string;
  suggestions: Suggestion[];
  port: number | null;
  dotenv: Record<string, string>;
  warnings: string[];
}

export interface Connection {
  env: Env;
  baseUrl: string;
  wsUrl: string;
  token: string | null;
  error: string | null;
  dataDir: string;
  version: string;
}

export class ApiError extends Error {}

let conn: Connection | null = null;

export async function connection(refresh = false): Promise<Connection> {
  if (!conn || refresh || !conn.token) {
    conn = await invoke<Connection>("agent_connection");
  }
  return conn;
}

async function call<T>(method: string, path: string, body?: unknown): Promise<T> {
  const c = await connection();
  if (!c.token) throw new ApiError(c.error ?? "No agent token.");
  let res: Response;
  try {
    res = await fetch(c.baseUrl + path, {
      method,
      headers: {
        Authorization: `Bearer ${c.token}`,
        ...(body ? { "Content-Type": "application/json" } : {}),
      },
      body: body ? JSON.stringify(body) : undefined,
    });
  } catch {
    throw new ApiError(
      c.env === "dev"
        ? `Cannot reach the dev agent at ${c.baseUrl}. Start it with "npm run dev:agent".`
        : `Cannot reach the Keeprun service at ${c.baseUrl}. Start the "Keeprun Agent" service in services.msc.`,
    );
  }
  if (res.status === 401) {
    // The agent may have been reinstalled with a new token.
    await connection(true);
  }
  if (!res.ok) {
    const msg = await res
      .json()
      .then((j) => j.error as string)
      .catch(() => res.statusText);
    throw new ApiError(msg);
  }
  return res.status === 204 ? (undefined as T) : res.json();
}

const enc = encodeURIComponent;

export const api = {
  list: () => call<AppView[]>("GET", "/apps"),
  add: (app: NewApp) => call<AppView>("POST", "/apps", app),
  update: (id: string, u: UpdateApp) => call<AppView>("PATCH", `/apps/${enc(id)}`, u),
  remove: (id: string) => call<void>("DELETE", `/apps/${enc(id)}`),
  start: (id: string) => call<AppView>("POST", `/apps/${enc(id)}/start`),
  stop: (id: string) => call<AppView>("POST", `/apps/${enc(id)}/stop`),
  restart: (id: string) => call<AppView>("POST", `/apps/${enc(id)}/restart`),
  events: (id: string, limit = 100) => call<AppEvent[]>("GET", `/apps/${enc(id)}/events?limit=${limit}`),
  async logSocket(id: string): Promise<WebSocket> {
    const c = await connection();
    return new WebSocket(`${c.wsUrl}/apps/${enc(id)}/logs?token=${c.token}`);
  },
};

export const desktop = {
  detect: (path: string) => invoke<Detection>("detect", { path }),
  readEnvFile: (path: string) => invoke<Record<string, string>>("read_env_file", { path }),
  looksSecret: (key: string) => invoke<boolean>("looks_secret", { key }),
};

export const config = {
  appEnv: import.meta.env.VITE_APP_ENV as Env,
  pollMs: Number(import.meta.env.VITE_STATUS_POLL_MS ?? 1000),
  logMaxLines: Number(import.meta.env.VITE_LOG_MAX_LINES ?? 5000),
};

export function formatDuration(ms: number): string {
  let s = Math.max(0, Math.floor(ms / 1000));
  const d = Math.floor(s / 86400);
  s %= 86400;
  const h = Math.floor(s / 3600);
  s %= 3600;
  const m = Math.floor(s / 60);
  s %= 60;
  if (d) return `${d}d ${h}h`;
  if (h) return `${h}h ${m}m`;
  if (m) return `${m}m ${s}s`;
  return `${s}s`;
}

export function formatBytes(b: number): string {
  if (b >= 1024 ** 3) return `${(b / 1024 ** 3).toFixed(1)} GB`;
  return `${Math.round(b / 1024 ** 2)} MB`;
}

export function formatTime(ts: number, withDate = false): string {
  const d = new Date(ts);
  const time = d.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });
  if (!withDate) return time;
  const today = new Date().toDateString() === d.toDateString();
  return today ? time : `${d.toLocaleDateString([], { month: "short", day: "numeric" })} ${time}`;
}

export function isActive(s: AppState): boolean {
  return s === "running" || s === "starting" || s === "stopping" || s === "restarting";
}

export const stateLabel: Record<AppState, string> = {
  running: "Running",
  starting: "Starting",
  stopping: "Stopping",
  stopped: "Stopped",
  exited: "Exited",
  restarting: "Restarting",
  failed: "Failed",
};

export function appUrl(app: AppView): string | null {
  const port = app.stats?.ports[0] ?? (app.env.PORT ? Number(app.env.PORT) : null);
  return port ? `http://localhost:${port}` : null;
}
