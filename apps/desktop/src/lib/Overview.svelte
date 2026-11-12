<script lang="ts">
  import { openPath, openUrl } from "@tauri-apps/plugin-opener";
  import { Bot, Copy, ExternalLink, FolderOpen, LoaderCircle, TriangleAlert } from "@lucide/svelte";
  import { api, appPort, desktop, formatBytes, formatDuration, formatTime, isActive, type AppView } from "./api";
  import { t } from "./i18n/index.svelte";
  import EventsList from "./EventsList.svelte";
  import GitInfo from "./GitInfo.svelte";
  import SourceText from "./SourceText.svelte";

  let {
    app,
    now,
    agentWorking = false,
    onshowevents,
  }: {
    app: AppView;
    now: number;
    /** A coding agent still works in the app's folder: errors wait until it finishes. */
    agentWorking?: boolean;
    onshowevents: () => void;
  } = $props();

  const running = $derived(app.state === "running");
  const urls = $derived(
    running && app.stats?.ports.length
      ? app.stats.ports.map((p) => `http://localhost:${p}`)
      : appPort(app)
        ? [`http://localhost:${appPort(app)}`]
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

  // Errors while running: copy a ready prompt for a coding agent, or dismiss.
  let issueError = $state("");
  let promptCopied = $state(false);
  async function copyPrompt() {
    issueError = "";
    try {
      const r = await api.issue(app.id);
      if (!r.prompt) return;
      await navigator.clipboard.writeText(r.prompt);
      promptCopied = true;
      setTimeout(() => (promptCopied = false), 2000);
    } catch (e) {
      issueError = (e as Error).message;
    }
  }
  async function dismissIssue() {
    issueError = "";
    try {
      await api.clearIssue(app.id);
    } catch (e) {
      issueError = (e as Error).message;
    }
  }

  // The linked env file changed while the app runs: offer the restart that applies it.
  const envFile = $derived(app.env_file_status);
  let envRestarting = $state(false);
  let envError = $state("");
  async function restartForEnv() {
    envError = "";
    envRestarting = true;
    try {
      await api.restart(app.id);
    } catch (e) {
      envError = (e as Error).message;
    } finally {
      envRestarting = false;
    }
  }

  // A copy started outside Keeprun, e.g. by a coding agent: offer to end it, and to tell
  // coding agents to use Keeprun so they do not start another one.
  let ending = $state(false);
  let outsideError = $state("");
  let noteFile = $state<string | null>(null);
  let noting = $state(false);
  // Each status poll is a new object: ask again only when the folder or the notice changes.
  const noteFolder = $derived(app.outside?.length ? app.cwd : null);
  $effect(() => {
    const cwd = noteFolder;
    if (!cwd) return;
    let live = true;
    desktop.agentNoteFile(cwd).then(
      (f) => live && (noteFile = f),
      () => {},
    );
    return () => (live = false);
  });
  async function endOutside() {
    outsideError = "";
    ending = true;
    try {
      await api.endOutside(app.id);
    } catch (e) {
      outsideError = (e as Error).message;
    } finally {
      ending = false;
    }
  }
  async function tellAgents() {
    outsideError = "";
    noting = true;
    try {
      noteFile = await desktop.addAgentNote(app.cwd, app.id);
    } catch (e) {
      outsideError = String(e);
    } finally {
      noting = false;
    }
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

  {#if app.env_file && envFile?.error}
    <p class="alert alert-warn mb-4 flex items-start gap-2" role="status">
      <TriangleAlert size={15} class="mt-0.5 shrink-0" aria-hidden="true" />
      <span class="selectable">{t("overview.envFileError", { error: envFile.error })}</span>
    </p>
  {:else if app.env_file && envFile?.changed.length}
    <div class="alert alert-warn mb-4 flex items-start gap-3" role="status">
      <TriangleAlert size={15} class="mt-0.5 shrink-0" aria-hidden="true" />
      <div class="min-w-0 flex-1">
        <p class="font-medium">{t("overview.envFileChanged", { file: app.env_file })}</p>
        <p class="selectable mt-1 truncate font-mono text-[12px]" dir="ltr">{envFile.changed.join(", ")}</p>
        {#if envError}<p class="mt-2 text-xs text-danger" role="alert">{envError}</p>{/if}
      </div>
      <button class="btn btn-primary shrink-0" onclick={restartForEnv} disabled={envRestarting}>{t("overview.restart")}</button>
    </div>
  {/if}

  {#if app.outside?.length}
    <div class="alert alert-warn mb-4 flex items-start gap-3" role="status">
      <TriangleAlert size={15} class="mt-0.5 shrink-0" aria-hidden="true" />
      <div class="min-w-0 flex-1">
        <p class="font-medium">{t("overview.outside")}</p>
        {#each app.outside as c (c.pid)}
          <p class="selectable mt-1 font-mono text-[12px]" dir="ltr">
            {c.name} · PID {c.pid}{#each c.ports as port} · {t("overview.outsidePort", { port })}{/each}
          </p>
        {/each}
        <p class="mt-1 text-xs opacity-80">{t("overview.outsideHint")}</p>
        <p class="mt-2 flex items-center gap-1.5 text-xs">
          <Bot size={13} class="shrink-0" aria-hidden="true" />
          {#if noteFile}
            {t("overview.agentsTold", { file: noteFile })}
          {:else}
            <button class="hover:underline" onclick={tellAgents} disabled={noting} title={t("overview.tellAgentsHint")}>{t("overview.tellAgents")}</button>
          {/if}
        </p>
        {#if outsideError}<p class="selectable mt-2 text-xs text-danger" role="alert">{outsideError}</p>{/if}
      </div>
      <button class="btn btn-primary shrink-0" onclick={endOutside} disabled={ending} title={t("overview.endOutsideHint")}>{t("overview.endOutside")}</button>
    </div>
  {/if}

  {#if app.issue && agentWorking}
    <p class="mb-4 flex items-center gap-2 text-xs text-fg-muted" role="status">
      <LoaderCircle size={13} class="shrink-0 animate-spin motion-reduce:animate-none" aria-hidden="true" />
      {t("overview.agentWorking")}
    </p>
  {:else if app.issue}
    <div class="alert alert-warn mb-4" role="status">
      <div class="flex items-start gap-3">
        <TriangleAlert size={15} class="mt-0.5 shrink-0" aria-hidden="true" />
        <div class="min-w-0 flex-1">
          <p class="font-medium">{t("overview.issue")}</p>
          <p class="selectable mt-1 truncate font-mono text-[12px]" dir="ltr" title={app.issue.summary}>{app.issue.summary}</p>
          <p class="mt-1 text-xs opacity-80">{t("overview.issueSeen", { count: app.issue.count, time: formatTime(app.issue.last_at, true) })}</p>
          {#if app.issue.excerpt.length}
            <details class="mt-2">
              <summary class="cursor-pointer text-xs">{t("overview.issueOutput")}</summary>
              <pre class="selectable mt-1 max-h-64 overflow-auto rounded border border-border bg-bg p-2 font-mono text-[11.5px] leading-[1.45] text-fg" dir="ltr">{#each app.issue.excerpt as line, i}{#if i}{"\n"}{/if}<SourceText text={line} cwd={app.cwd} onerror={(m) => (openError = m)} />{/each}</pre>
            </details>
          {/if}
          {#if issueError}<p class="mt-2 text-xs text-danger" role="alert">{issueError}</p>{/if}
        </div>
        <div class="flex shrink-0 flex-col items-end gap-1.5">
          <button class="btn btn-primary" onclick={copyPrompt} title={t("overview.copyPromptHint")}>
            <Copy size={13} />
            {promptCopied ? t("common.copied") : t("overview.copyPrompt")}
          </button>
          <button class="text-xs hover:underline" onclick={dismissIssue}>{t("overview.dismiss")}</button>
        </div>
      </div>
    </div>
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
    <GitInfo {app} />
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
