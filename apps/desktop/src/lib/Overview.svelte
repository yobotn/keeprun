<script lang="ts">
  import { openPath, openUrl } from "@tauri-apps/plugin-opener";
  import { Copy, ExternalLink, FolderOpen } from "@lucide/svelte";
  import { formatBytes, formatDuration, isActive, type AppView } from "./api";
  import { t } from "./i18n/index.svelte";
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
  let openError = $state("");

  // A refused or failed open must say so, not do nothing.
  async function open(fn: () => Promise<void>, what: string) {
    openError = "";
    try {
      await fn();
    } catch (e) {
      openError = t("overview.cannotOpen", { what, error: String(e) });
    }
  }

  function copy(text: string) {
    navigator.clipboard.writeText(text);
    copied = text;
    setTimeout(() => (copied = ""), 1200);
  }

  const policy = $derived({
    never: t("overview.policyNever"),
    "on-failure": t("overview.policyOnFailure"),
    always: t("overview.policyAlways"),
  });
  const seconds = $derived(app.next_restart_at ? Math.max(0, Math.ceil((app.next_restart_at - now) / 1000)) : 0);
</script>

<div class="flex-1 overflow-auto px-6 py-5">
  {#if app.state === "restarting" && app.next_restart_at}
    <p class="alert alert-warn mb-4" role="status">
      {t(app.max_restarts ? "overview.restartingOf" : "overview.restarting", {
        code: app.exit_code ?? t("overview.unknownCode"),
        seconds,
        attempt: app.restarts,
        max: app.max_restarts,
      })}
    </p>
  {:else if app.state === "failed"}
    <p class="alert mb-4" role="alert">
      {t("overview.failed", { max: app.max_restarts })}
    </p>
  {/if}

  <div class="grid grid-cols-4 gap-3">
    {#each [
      [t("overview.cpu"), running && app.stats ? `${app.stats.cpu_percent.toFixed(0)}%` : "-"],
      [t("overview.memory"), running && app.stats ? formatBytes(app.stats.memory_bytes) : "-"],
      [t("overview.uptime"), isActive(app.state) && app.started_at ? formatDuration(now - app.started_at) : "-"],
      [t("overview.processes"), running && app.stats ? String(app.stats.processes) : "-"],
    ] as [label, value]}
      <div class="rounded-lg border border-border px-4 py-3">
        <div class="text-xs text-fg-muted">{label}</div>
        <div class="mt-1 text-lg font-semibold tabular-nums">{value}</div>
      </div>
    {/each}
  </div>

  {#if openError}<p class="selectable alert mt-4" role="alert">{openError}</p>{/if}

  <dl class="mt-6 grid grid-cols-[minmax(110px,auto)_1fr] items-center gap-x-4 gap-y-3 text-[13px]">
    {#if urls.length}
      <dt class="text-fg-muted">{t("overview.url")}</dt>
      <dd class="flex flex-wrap items-center gap-3">
        {#each urls as url}
          <span class="inline-flex items-center gap-1">
            <button class="font-medium text-accent hover:underline" dir="ltr" onclick={() => open(() => openUrl(url), url)} title={t("overview.openBrowser")}>
              {url}
            </button>
            <button class="btn-icon !size-6" onclick={() => open(() => openUrl(url), url)} title={t("overview.openBrowser")}><ExternalLink size={13} /></button>
            <button class="btn-icon !size-6" onclick={() => copy(url)} title={t("overview.copyUrl")}><Copy size={13} /></button>
            {#if copied === url}<span class="text-xs text-fg-muted">{t("common.copied")}</span>{/if}
          </span>
        {/each}
        {#if !running}<span class="text-xs text-fg-muted">{t("overview.notListening")}</span>{/if}
      </dd>
    {/if}
    <dt class="text-fg-muted">{t("overview.folder")}</dt>
    <dd class="flex min-w-0 items-center gap-2">
      <span class="selectable truncate font-mono text-[12.5px]">{app.cwd}</span>
      <button class="btn-icon !size-6" onclick={() => open(() => openPath(app.cwd), t("overview.theFolder"))} title={t("overview.openFolder")}><FolderOpen size={13} /></button>
    </dd>
    <dt class="text-fg-muted">{t("overview.command")}</dt>
    <dd class="selectable break-all font-mono text-[12.5px]">{app.command}</dd>
    <dt class="text-fg-muted">{t("overview.pid")}</dt>
    <dd class="tabular-nums">{app.pid ?? "-"}</dd>
    <dt class="text-fg-muted">{t("overview.restart")}</dt>
    <dd>
      {app.restart === "never"
        ? policy.never
        : app.max_restarts
          ? t("overview.upTo", { policy: policy[app.restart], count: app.max_restarts })
          : t("overview.upToUnlimited", { policy: policy[app.restart] })}
    </dd>
    <dt class="text-fg-muted">{t("overview.autostart")}</dt>
    <dd>{app.autostart ? t("overview.autostartYes") : t("common.no")}</dd>
    {#if app.tags.length}
      <dt class="text-fg-muted">{t("overview.tags")}</dt>
      <dd class="flex gap-1.5">
        {#each app.tags as t}<span class="rounded bg-bg-hover px-1.5 py-0.5 text-xs">{t}</span>{/each}
      </dd>
    {/if}
  </dl>

  <div class="mt-8 flex items-center justify-between">
    <h2 class="text-sm font-semibold">{t("overview.recentEvents")}</h2>
    <button class="text-xs text-accent hover:underline" onclick={onshowevents}>{t("overview.allEvents")}</button>
  </div>
  <div class="mt-2">
    <EventsList appId={app.id} limit={6} compact />
  </div>
</div>
