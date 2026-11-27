// Desktop preferences remembered per user: the notification area icon, and the last app and
// tab the user looked at.
import { desktop, type TrayText } from "./api";
import { t } from "./i18n/index.svelte";

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    // Storage can be unavailable. Use the default.
    return null;
  }
}

function write(key: string, value: string | null) {
  try {
    if (value === null) localStorage.removeItem(key);
    else localStorage.setItem(key, value);
  } catch {
    // Not persisted. The choice still applies for this session.
  }
}

export const editors = ["auto", "vscode", "cursor", "windsurf", "default"] as const;
export type Editor = (typeof editors)[number];

/** Minutes between background Git fetches. Declared before `prefs`, which reads it. */
export const fetchIntervals = [0, 1, 5, 15, 30] as const;

export const prefs = $state({
  /** Keep Keeprun in the notification area when the window closes. On by default. */
  tray: read("keeprun.tray") !== "off",
  lastApp: read("keeprun.lastApp"),
  lastTab: read("keeprun.lastTab"),
  /** Sidebar sections the user collapsed. */
  collapsed: (read("keeprun.collapsed") ?? "").split(",").filter(Boolean),
  /** Where file paths in app output open: the first editor found, one editor, or the default app. */
  editor: readEditor(),
  /** Minutes between background Git fetches. 0, the default: only when the user clicks refresh.
   *  A fetch writes in the project's .git, so Keeprun does it only when allowed. */
  gitFetchMinutes: readFetchMinutes(),
  /** Windows notification when a background check finds commits to pull. On by default. */
  gitNotify: read("keeprun.gitNotify") !== "off",
});

function readFetchMinutes(): number {
  const v = Number(read("keeprun.gitFetchMinutes") ?? 0);
  return (fetchIntervals as readonly number[]).includes(v) ? v : 0;
}

export function setFetchMinutes(minutes: number) {
  prefs.gitFetchMinutes = minutes;
  write("keeprun.gitFetchMinutes", String(minutes));
}

export function setGitNotify(on: boolean) {
  prefs.gitNotify = on;
  write("keeprun.gitNotify", on ? "on" : "off");
}

/** Send the background Git check settings and notification text to the shell. */
export function syncGitWatch() {
  const text = { title: t("notify.gitTitle", { name: "{name}" }), body: t("notify.gitBody", { count: "{count}", upstream: "{upstream}" }) };
  desktop.setGitWatch(prefs.gitFetchMinutes, prefs.gitNotify, text).catch(() => {});
}

function readEditor(): Editor {
  const v = read("keeprun.editor");
  return editors.find((e) => e === v) ?? "auto";
}

export function setEditor(editor: Editor) {
  prefs.editor = editor;
  write("keeprun.editor", editor);
}

export function setTray(on: boolean) {
  prefs.tray = on;
  write("keeprun.tray", on ? "on" : "off");
  desktop.setTrayEnabled(on).catch(() => {});
}

export function rememberSelection(app: string | null, tab: string) {
  if (app !== prefs.lastApp) write("keeprun.lastApp", (prefs.lastApp = app));
  if (tab !== prefs.lastTab) write("keeprun.lastTab", (prefs.lastTab = tab));
}

export function toggleSection(id: string) {
  prefs.collapsed = prefs.collapsed.includes(id) ? prefs.collapsed.filter((c) => c !== id) : [...prefs.collapsed, id];
  write("keeprun.collapsed", prefs.collapsed.join(","));
}

/** Send translated tray text to the shell. Counts cover 1..`max` so plurals are right. */
export function syncTrayText(max: number) {
  const counts = Array.from({ length: Math.max(1, max) }, (_, i) => i + 1);
  const text: TrayText = {
    show: t("tray.show"),
    quit: t("tray.quit"),
    summary: t("tray.summary", { running: "{running}", total: "{total}" }),
    noApps: t("tray.noApps"),
    unreachable: t("tray.unreachable"),
    failed: Object.fromEntries(counts.map((n) => [n, t("tray.failed", { count: n })])),
    restarting: Object.fromEntries(counts.map((n) => [n, t("tray.restarting", { count: n })])),
    errors: Object.fromEntries(counts.map((n) => [n, t("tray.errors", { count: n })])),
    unhealthy: Object.fromEntries(counts.map((n) => [n, t("tray.unhealthy", { count: n })])),
    copyPrompt: t("tray.copyPrompt", { name: "{name}" }),
    copied: t("tray.copied"),
  };
  desktop.setTrayText(text).catch(() => {});
}
