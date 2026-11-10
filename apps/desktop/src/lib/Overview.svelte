<script lang="ts">
  import { openPath, openUrl } from "@tauri-apps/plugin-opener";
  import { Copy, ExternalLink, FolderOpen } from "@lucide/svelte";
  import { formatBytes, formatDuration, isActive, type AppView } from "./api";
  import EventsList from "./EventsList.svelte";

  let { app, now, onshowevents }: { app: AppView; now: number; onshowevents: () => void } = $props();

  const running = $derived(app.state === "running");
  const urls = $derived(
    running && app.stats?.ports.length
      ? app.stats.ports.map((p) => `http://localhost:${p}`)
      : app.env.PORT
        ? [`http://localhost:${app.env.PORT}`]
        : [],
  );
  let copied = $state("");

  function copy(text: string) {
    navigator.clipboard.writeText(text);
    copied = text;
    setTimeout(() => (copied = ""), 1200);
  }

  const policy = { never: "Never", "on-failure": "On failure", always: "Always" };
</script>

<div class="flex-1 overflow-auto px-6 py-5">
  {#if app.state === "restarting" && app.next_restart_at}
    <p class="alert alert-warn mb-4" role="status">
      Exited with code {app.exit_code ?? "unknown"}. Restarting in {Math.max(0, Math.ceil((app.next_restart_at - now) / 1000))}s
      (attempt {app.restarts}{app.max_restarts ? ` of ${app.max_restarts}` : ""}).
    </p>
  {:else if app.state === "failed"}
    <p class="alert mb-4" role="alert">
      Stopped restarting after {app.max_restarts} crashes in a row. Check the logs, fix the cause, then Start.
    </p>
  {/if}

  <div class="grid grid-cols-4 gap-3">
    {#each [
      ["CPU", running && app.stats ? `${app.stats.cpu_percent.toFixed(0)}%` : "-"],
      ["Memory", running && app.stats ? formatBytes(app.stats.memory_bytes) : "-"],
      ["Uptime", isActive(app.state) && app.started_at ? formatDuration(now - app.started_at) : "-"],
      ["Processes", running && app.stats ? String(app.stats.processes) : "-"],
    ] as [label, value]}
      <div class="rounded-lg border border-border px-4 py-3">
        <div class="text-xs text-fg-muted">{label}</div>
        <div class="mt-1 text-lg font-semibold tabular-nums">{value}</div>
      </div>
    {/each}
  </div>

  <dl class="mt-6 grid grid-cols-[110px_1fr] items-center gap-x-4 gap-y-3 text-[13px]">
    {#if urls.length}
      <dt class="text-fg-muted">URL</dt>
      <dd class="flex flex-wrap items-center gap-3">
        {#each urls as url}
          <span class="inline-flex items-center gap-1">
            <button class="font-medium text-accent hover:underline" onclick={() => openUrl(url)} title="Open in browser">
              {url}
            </button>
            <button class="btn-icon !size-6" onclick={() => openUrl(url)} title="Open in browser"><ExternalLink size={13} /></button>
            <button class="btn-icon !size-6" onclick={() => copy(url)} title="Copy URL"><Copy size={13} /></button>
            {#if copied === url}<span class="text-xs text-fg-muted">Copied</span>{/if}
          </span>
        {/each}
        {#if !running}<span class="text-xs text-fg-muted">from PORT, not listening</span>{/if}
      </dd>
    {/if}
    <dt class="text-fg-muted">Folder</dt>
    <dd class="flex min-w-0 items-center gap-2">
      <span class="selectable truncate font-mono text-[12.5px]">{app.cwd}</span>
      <button class="btn-icon !size-6" onclick={() => openPath(app.cwd)} title="Open folder"><FolderOpen size={13} /></button>
    </dd>
    <dt class="text-fg-muted">Command</dt>
    <dd class="selectable break-all font-mono text-[12.5px]">{app.command}</dd>
    <dt class="text-fg-muted">PID</dt>
    <dd class="tabular-nums">{app.pid ?? "-"}</dd>
    <dt class="text-fg-muted">Restart</dt>
    <dd>
      {policy[app.restart]}{app.restart !== "never" ? `, up to ${app.max_restarts || "unlimited"} times in a row` : ""}
    </dd>
    <dt class="text-fg-muted">Start with agent</dt>
    <dd>{app.autostart ? "Yes, unless stopped by a user" : "No"}</dd>
    {#if app.tags.length}
      <dt class="text-fg-muted">Tags</dt>
      <dd class="flex gap-1.5">
        {#each app.tags as t}<span class="rounded bg-bg-hover px-1.5 py-0.5 text-xs">{t}</span>{/each}
      </dd>
    {/if}
  </dl>

  <div class="mt-8 flex items-center justify-between">
    <h2 class="text-sm font-semibold">Recent events</h2>
    <button class="text-xs text-accent hover:underline" onclick={onshowevents}>All events</button>
  </div>
  <div class="mt-2">
    <EventsList appId={app.id} limit={6} compact />
  </div>
</div>
