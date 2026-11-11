// Update state shared by the header button, the update dialog and the agent dialog.
import { Channel, invoke } from "@tauri-apps/api/core";

export interface UpdatePolicy {
  enabled: boolean;
  reason: string | null;
  url: string | null;
}

export interface UpdateInfo {
  version: string;
  currentVersion: string;
  notes: string | null;
  date: string | null;
}

type Progress =
  | { event: "started"; data: { total: number | null } }
  | { event: "chunk"; data: { downloaded: number } }
  | { event: "installing" };

export const updates = $state({
  policy: null as UpdatePolicy | null,
  available: null as UpdateInfo | null,
  checking: false,
  /** Set after a manual check finds nothing. */
  upToDate: false,
  error: "",
  installing: false,
  downloaded: 0,
  total: null as number | null,
  phase: "" as "" | "downloading" | "installing",
});

const SIX_HOURS = 6 * 60 * 60 * 1000;

export async function checkForUpdates(manual = false) {
  if (updates.checking) return;
  updates.checking = true;
  updates.error = "";
  updates.upToDate = false;
  try {
    updates.policy ??= await invoke<UpdatePolicy>("update_policy");
    if (!updates.policy.enabled) return;
    updates.available = await invoke<UpdateInfo | null>("check_update");
    updates.upToDate = manual && !updates.available;
  } catch (e) {
    // Background checks fail quietly (offline servers are normal). Manual checks show why.
    if (manual) updates.error = String(e);
  } finally {
    updates.checking = false;
  }
}

/** Check shortly after start, then every six hours. Returns a cleanup function. */
export function scheduleUpdateChecks(): () => void {
  const first = setTimeout(() => checkForUpdates(), 10_000);
  const every = setInterval(() => checkForUpdates(), SIX_HOURS);
  return () => {
    clearTimeout(first);
    clearInterval(every);
  };
}

export async function installUpdate() {
  updates.installing = true;
  updates.error = "";
  updates.downloaded = 0;
  updates.total = null;
  updates.phase = "downloading";
  const onProgress = new Channel<Progress>();
  onProgress.onmessage = (m) => {
    if (m.event === "started") updates.total = m.data.total;
    else if (m.event === "chunk") updates.downloaded = m.data.downloaded;
    else updates.phase = "installing";
  };
  try {
    await invoke("install_update", { onProgress });
  } catch (e) {
    updates.error = String(e);
    updates.phase = "";
    // The pending update was consumed. Look it up again so the user can retry.
    updates.available = null;
    checkForUpdates();
  } finally {
    updates.installing = false;
  }
}
