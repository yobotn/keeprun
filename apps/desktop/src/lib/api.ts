// Client for the local Keeprun agent API, plus desktop-side commands.
import { invoke } from "@tauri-apps/api/core";
import { intlLocale, t } from "./i18n/index.svelte";

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
  | "failed"
  | "task-started"
  | "task-finished"
  | "task-failed";

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
  tasks: TaskSpec[];
  /** Kept but switched off: cannot start, never autostarts. */
  disabled: boolean;
  /** Watch the output for errors while the app runs. */
  watch_errors: boolean;
  /** Env file read on every start, relative to `cwd` or absolute. `env` overrides it. */
  env_file?: string | null;
}

/** The linked env file as the agent sees it. Keys only: values may be secret. */
export interface EnvFileStatus {
  keys: string[];
  port: number | null;
  /** Keys changed since the running process started. Restart to apply. */
  changed: string[];
  /** Why the file cannot be read, e.g. it was deleted. */
  error: string | null;
}

/** Errors an app printed while it kept running, e.g. a dev server whose build broke. */
export interface AppIssue {
  first_at: number;
  last_at: number;
  /** Times the first error was printed. */
  count: number;
  /** The first error line. */
  summary: string;
  /** Output around the first occurrence, with repeats removed. */
  excerpt: string[];
}

export interface IssueReport {
  issue: AppIssue | null;
  /** Text for a coding agent, when there is an issue. */
  prompt: string | null;
}

export interface TaskSpec {
  name: string;
  command: string;
}

export interface TaskRun {
  name: string | null;
  command: string;
  running: boolean;
  started_at: number;
  finished_at: number | null;
  exit_code: number | null;
}

export interface AppView extends AppSpec {
  state: AppState;
  pid: number | null;
  exit_code: number | null;
  started_at: number | null;
  restarts: number;
  next_restart_at: number | null;
  stats: Stats | null;
  task: TaskRun | null;
  issue: AppIssue | null;
  env_file_status: EnvFileStatus | null;
}

export type NewApp = Omit<AppSpec, "id" | "disabled" | "watch_errors"> & { watch_errors?: boolean };
/** `env_file: ""` unlinks the env file. */
export type UpdateApp = Partial<NewApp> & { disabled?: boolean };

/** PORT the app gets: set in Keeprun, else from its env file. */
export function appPort(app: AppView): string | null {
  return app.env.PORT ?? (app.env_file_status?.port ? String(app.env_file_status.port) : null);
}

export interface LogLine {
  ts: number;
  stream: LogStream;
  text: string;
  /** Output of a task rather than the app. */
  task?: boolean;
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
  /** Short task name, e.g. "db:migrate". */
  name: string;
}

export interface Detection {
  kind: "node" | "dotnet" | "python" | "custom";
  name: string;
  suggestions: Suggestion[];
  tasks: Suggestion[];
  port: number | null;
  /** `.env` and `.env.*` files in the folder. `.env` first, templates last. */
  env_files: EnvFile[];
  warnings: string[];
}

export interface EnvFile {
  /** File name, e.g. ".env.local". */
  name: string;
  vars: Record<string, string>;
  /** A placeholder file such as `.env.example`. Never selected by default. */
  template: boolean;
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
  if (!c.token) throw new ApiError(c.error ?? t("error.noToken"));
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
    // Settings may have changed, e.g. a new port. Read them again on the next call.
    conn = null;
    throw new ApiError(
      t(c.env === "dev" ? "error.unreachableDev" : "error.unreachableProd", { url: c.baseUrl }),
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
  runTask: (id: string, task: { name?: string; command?: string }) =>
    call<AppView>("POST", `/apps/${enc(id)}/tasks/run`, task),
  stopTask: (id: string) => call<AppView>("POST", `/apps/${enc(id)}/tasks/stop`),
  issue: (id: string) => call<IssueReport>("GET", `/apps/${enc(id)}/issue`),
  clearIssue: (id: string) => call<AppView>("DELETE", `/apps/${enc(id)}/issue`),
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
  /** Change the agent port. In prod, Windows asks for administrator approval. */
  setAgentPort: (port: number) => invoke<string>("set_agent_port", { port }),
  /** Show the notification area icon, and hide to it on close. */
  setTrayEnabled: (enabled: boolean) => invoke<void>("set_tray_enabled", { enabled }),
  setTrayText: (text: TrayText) => invoke<void>("set_tray_text", { text }),
  /** Folder Keeprun was started with from the Explorer menu, once. */
  takeLaunchFolder: () => invoke<string | null>("take_launch_folder"),
};

export interface TrayText {
  show: string;
  quit: string;
  summary: string;
  noApps: string;
  unreachable: string;
  /** Count to text, e.g. 1 -> "1 app failed". */
  failed: Record<number, string>;
  restarting: Record<number, string>;
  errors: Record<number, string>;
  /** "Copy fix prompt for {name}" */
  copyPrompt: string;
  /** "{name} has errors" */
  issueTitle: string;
  copied: string;
}

export const config = {
  appEnv: import.meta.env.VITE_APP_ENV as Env,
  pollMs: Number(import.meta.env.VITE_STATUS_POLL_MS ?? 1000),
  logMaxLines: Number(import.meta.env.VITE_LOG_MAX_LINES ?? 5000),
};

// Intl.DurationFormat is in WebView2 (Chromium 129+). TypeScript does not type it yet.
type DurationFormat = { format(d: Record<string, number>): string };
const DurationFormatCtor = (Intl as unknown as { DurationFormat?: new (l: string, o: object) => DurationFormat })
  .DurationFormat;

export function formatDuration(ms: number): string {
  let s = Math.max(0, Math.floor(ms / 1000));
  const d = Math.floor(s / 86400);
  s %= 86400;
  const h = Math.floor(s / 3600);
  s %= 3600;
  const m = Math.floor(s / 60);
  s %= 60;
  // Two most significant units, like "2d 3h" or "4m 10s".
  const parts: Record<string, number> = d ? { days: d, hours: h } : h ? { hours: h, minutes: m } : m ? { minutes: m, seconds: s } : { seconds: s };
  if (DurationFormatCtor) {
    try {
      return new DurationFormatCtor(intlLocale(), { style: "narrow" }).format(parts);
    } catch {
      // Fall through to the plain format.
    }
  }
  if (d) return `${d}d ${h}h`;
  if (h) return `${h}h ${m}m`;
  if (m) return `${m}m ${s}s`;
  return `${s}s`;
}

export function formatBytes(b: number): string {
  const nf = (v: number, digits: number) =>
    new Intl.NumberFormat(intlLocale(), { maximumFractionDigits: digits, minimumFractionDigits: digits }).format(v);
  if (b >= 1024 ** 3) return `${nf(b / 1024 ** 3, 1)} GB`;
  return `${nf(Math.round(b / 1024 ** 2), 0)} MB`;
}

export function formatTime(ts: number, withDate = false): string {
  const d = new Date(ts);
  const loc = intlLocale();
  const time = d.toLocaleTimeString(loc, { hour: "2-digit", minute: "2-digit", second: "2-digit", hour12: false });
  if (!withDate) return time;
  const today = new Date().toDateString() === d.toDateString();
  return today ? time : `${d.toLocaleDateString(loc, { month: "short", day: "numeric" })} ${time}`;
}

export function formatDate(ts: number | string): string {
  return new Date(ts).toLocaleDateString(intlLocale(), { year: "numeric", month: "short", day: "numeric" });
}

export function isActive(s: AppState): boolean {
  return s === "running" || s === "starting" || s === "stopping" || s === "restarting";
}

export function stateText(s: AppState): string {
  return t(`state.${s}`);
}

/** Split a tag list on Latin, Arabic and CJK commas. */
export function splitTags(text: string): string[] {
  return text
    .split(/[,،，、]/)
    .map((t) => t.trim())
    .filter(Boolean);
}

/** Compare Windows paths: case, slash style and a trailing slash do not matter. */
export function samePath(a: string, b: string): boolean {
  const norm = (p: string) => p.trim().replace(/\//g, "\\").replace(/\\+$/, "").toLowerCase();
  return norm(a) === norm(b);
}

export function appUrl(app: AppView): string | null {
  const port = app.stats?.ports[0] ?? (appPort(app) ? Number(appPort(app)) : null);
  return port ? `http://localhost:${port}` : null;
}
