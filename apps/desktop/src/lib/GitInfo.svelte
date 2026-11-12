<script lang="ts">
  // Git state of the app folder, and a pull that brings the app up to date: fast-forward,
  // install dependencies when a lockfile changed, restart when it runs. Rendered as one row of
  // the Overview details list. Nothing shows when the folder is not in a Git repository.
  import { ArrowDown, ArrowUp, Download, FilePen, GitBranch, LoaderCircle } from "@lucide/svelte";
  import { api, desktop, formatTime, isActive, type AppView, type GitStatus } from "./api";
  import { t } from "./i18n/index.svelte";

  let { app }: { app: AppView } = $props();

  let git = $state<GitStatus | null>(null);
  let pulling = $state(false);
  let result = $state("");
  let error = $state("");

  // Each status poll is a new app object: follow the folder only.
  const folder = $derived(app.cwd);

  $effect(() => {
    const cwd = folder;
    let live = true;
    git = null;
    result = "";
    error = "";
    const local = () => desktop.gitStatus(cwd).then((s) => live && (git = s), () => {});
    // Fetch quietly, without asking for credentials, so "behind" is current.
    const fetch = () => desktop.gitFetch(cwd).then((s) => live && (git = s), () => {});
    local().then(fetch);
    const quick = setInterval(local, 5000);
    const slow = setInterval(fetch, 5 * 60 * 1000);
    return () => {
      live = false;
      clearInterval(quick);
      clearInterval(slow);
    };
  });

  /** Wait for the task started at or after `since` to end. True when it succeeded. */
  async function taskSucceeded(id: string, since: number): Promise<boolean> {
    for (let i = 0; i < 1200; i++) {
      await new Promise((r) => setTimeout(r, 500));
      const run = (await api.get(id)).task;
      if (run && run.started_at >= since && !run.running) return run.exit_code === 0;
    }
    return false;
  }

  async function pull() {
    const id = app.id;
    const cwd = app.cwd;
    const restart = isActive(app.state);
    const install = app.tasks.some((task) => task.name === "install");
    pulling = true;
    result = "";
    error = "";
    try {
      const r = await desktop.gitPull(cwd);
      git = r.status;
      if (r.commits === 0) {
        result = t("git.upToDate");
        return;
      }
      const parts = [t("git.pulled", { count: r.commits })];
      if (r.dependencies && install) {
        const since = Date.now() - 1000;
        await api.runTask(id, { name: "install" });
        if (!(await taskSucceeded(id, since))) {
          error = [...parts, t("git.installFailed")].join(" ");
          return;
        }
        parts.push(t("git.installed"));
      } else if (r.dependencies) {
        parts.push(t("git.noInstallTask"));
      }
      if (restart) {
        await api.restart(id);
        parts.push(t("git.restarted"));
      }
      result = parts.join(" ");
    } catch (e) {
      error = String((e as Error)?.message ?? e);
    } finally {
      pulling = false;
    }
  }
</script>

{#if git}
  <dt class="text-fg-muted">{t("git.label")}</dt>
  <dd class="min-w-0">
    <div class="flex flex-wrap items-center gap-x-3 gap-y-1">
      <span class="inline-flex min-w-0 items-center gap-1.5 font-medium" dir="ltr">
        <GitBranch size={13} class="shrink-0 text-fg-muted" aria-hidden="true" />
        <span class="selectable truncate">{git.branch ?? t("git.detached")}</span>
      </span>
      {#if git.upstream && (git.ahead || git.behind)}
        <span class="inline-flex items-center gap-2 text-xs tabular-nums text-fg-muted" dir="ltr">
          {#if git.behind}
            <span class="inline-flex items-center gap-0.5 text-warn" title={t("git.behindHint", { count: git.behind, upstream: git.upstream })}>
              <ArrowDown size={12} aria-hidden="true" />{git.behind}
            </span>
          {/if}
          {#if git.ahead}
            <span class="inline-flex items-center gap-0.5" title={t("git.aheadHint", { count: git.ahead, upstream: git.upstream })}>
              <ArrowUp size={12} aria-hidden="true" />{git.ahead}
            </span>
          {/if}
        </span>
      {/if}
      {#if git.changed}
        <span class="inline-flex items-center gap-0.5 text-xs tabular-nums text-fg-muted" title={t("git.changedHint", { count: git.changed })}>
          <FilePen size={12} aria-hidden="true" />{git.changed}
        </span>
      {/if}
      {#if git.upstream}
        <button
          class="btn !h-7 !px-2 text-xs"
          onclick={pull}
          disabled={pulling}
          title={t("git.pullHint", { upstream: git.upstream })}
        >
          {#if pulling}
            <LoaderCircle size={13} class="animate-spin motion-reduce:animate-none" aria-hidden="true" />
          {:else}
            <Download size={13} aria-hidden="true" />
          {/if}
          {t(isActive(app.state) ? "git.pullRestart" : "git.pull")}
        </button>
      {:else}
        <span class="text-xs text-fg-muted">{t("git.noUpstream")}</span>
      {/if}
    </div>
    {#if git.commit}
      <p class="mt-1 truncate text-xs text-fg-muted" title="{git.commit.subject}, {git.commit.author}">
        <span class="font-mono" dir="ltr">{git.commit.hash}</span>
        <span class="selectable">{git.commit.subject}</span>
        · {formatTime(git.commit.time, true)}
      </p>
    {/if}
    {#if result}<p class="mt-1 text-xs text-fg-muted" role="status">{result}</p>{/if}
    {#if error}<p class="selectable mt-1 text-xs text-danger" role="alert">{error}</p>{/if}
  </dd>
{/if}
