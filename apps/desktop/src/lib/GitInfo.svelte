<script lang="ts">
  // Git state of the app folder, and a pull that brings the app up to date: fast-forward,
  // install dependencies when a lockfile changed, migrate when a migration changed, restart
  // when it runs. Rendered as one row of the Overview details list. Nothing shows when the
  // folder is not in a Git repository. Reading the state never writes in the repository.
  // A fetch runs on the refresh click, or on open and focus only when background checks are on
  // in Preferences. Nothing changes the working tree except a pull the user clicks. States that
  // need the developer (diverged, not pushed, remote branch deleted) offer a fix prompt for a
  // coding agent instead of acting.
  import {
    CircleAlert,
    CircleCheck,
    Copy,
    Download,
    FilePen,
    GitBranch,
    GitMerge,
    LoaderCircle,
    RefreshCw,
  } from "@lucide/svelte";
  import { listen } from "@tauri-apps/api/event";
  import { api, desktop, formatTime, isActive, samePath, timeAgo, type AppView, type GitChecked, type GitStatus } from "./api";
  import { intlLocale, t } from "./i18n/index.svelte";
  import { prefs } from "./prefs.svelte";

  let { app }: { app: AppView } = $props();

  let git = $state<GitStatus | null>(null);
  let pulling = $state(false);
  let fetching = $state(false);
  let fetchedAt = $state<number | null>(null);
  let fetchError = $state("");
  let now = $state(Date.now());
  let result = $state("");
  let error = $state("");

  // Each status poll is a new app object: follow the folder only.
  const folder = $derived(app.cwd);

  /** A fetch on open or focus is skipped when the last one is this recent. */
  const FRESH = 60_000;

  /** The shell's record of the last fetch: when it worked, or why it failed. On a failure
   *  the last known state stays, marked as stale. */
  function apply(c: GitChecked | null) {
    if (!c || !samePath(c.cwd, folder)) return;
    fetchedAt = c.at;
    fetchError = c.error?.trim() ?? "";
    if (c.status && !c.error) git = c.status;
  }

  /** Fetch, then the state. One at a time: a click while one runs does nothing. */
  async function refresh() {
    if (fetching) return;
    const cwd = folder;
    fetching = true;
    try {
      const s = await desktop.gitFetch(cwd);
      if (samePath(cwd, folder)) git = s;
    } catch {
      // The shell recorded the error. It shows below.
    } finally {
      fetching = false;
    }
    apply(await desktop.gitChecked(cwd).catch(() => null));
  }

  /** On open and focus: only when the user allowed background checks. */
  function refreshIfOld() {
    if (!prefs.gitFetchMinutes) return;
    if (!fetchedAt || Date.now() - fetchedAt > FRESH) refresh();
  }

  $effect(() => {
    const cwd = folder;
    let live = true;
    git = null;
    result = "";
    error = "";
    fetchedAt = null;
    fetchError = "";
    const local = () => desktop.gitStatus(cwd).then((s) => live && (git = s), () => {});
    desktop
      .gitChecked(cwd)
      .then(apply, () => {})
      .then(local)
      .then(() => live && git && refreshIfOld());
    const quick = setInterval(() => {
      local();
      now = Date.now();
    }, 5000);
    const onFocus = () => git && refreshIfOld();
    window.addEventListener("focus", onFocus);
    // Background checks run in the shell, as often as the preference says.
    let unlisten: (() => void) | undefined;
    listen<GitChecked>("git-checked", (e) => live && apply(e.payload)).then((un) => (live ? (unlisten = un) : un()));
    return () => {
      live = false;
      clearInterval(quick);
      window.removeEventListener("focus", onFocus);
      unlisten?.();
    };
  });

  /** Remote of the upstream, or of the default branch. */
  const remote = $derived((git?.upstream ?? git?.default_branch ?? "").split("/")[0] || "origin");
  const mainBranch = $derived(git?.default_branch?.slice(remote.length + 1) ?? "main");

  type Tone = "ok" | "warn" | "muted";
  const label = $derived.by((): { text: string; hint: string; tone: Tone } | null => {
    if (!git) return null;
    const upstream = git.upstream ?? "";
    switch (git.sync) {
      case "in_sync":
        return { text: t("git.inSync"), hint: t("git.inSyncHint", { upstream }), tone: "muted" };
      case "behind":
        return { text: t("git.needsPull", { count: git.behind }), hint: t("git.behindHint", { count: git.behind, upstream }), tone: "warn" };
      case "ahead":
        return { text: t("git.needsPush", { count: git.ahead }), hint: t("git.aheadHint", { count: git.ahead, upstream }), tone: "muted" };
      case "diverged":
        return { text: t("git.diverged", { ahead: git.ahead, behind: git.behind }), hint: t("git.divergedHint"), tone: "warn" };
      case "merged":
        return { text: t("git.merged", { branch: mainBranch }), hint: t("git.mergedHint", { branch: mainBranch }), tone: "ok" };
      case "gone":
        return { text: t("git.gone"), hint: t("git.goneHint", { upstream }), tone: "muted" };
      case "no_upstream":
        return { text: t("git.noUpstream"), hint: "", tone: "muted" };
      case "no_remote":
        return { text: t("git.noRemote"), hint: "", tone: "muted" };
      default:
        return null;
    }
  });
  const toneClass: Record<Tone, string> = { ok: "text-accent", warn: "text-warn", muted: "text-fg-muted" };
  const canFetch = $derived(git !== null && git.sync !== "no_remote");

  /** A prompt for the user's coding agent when the branch needs the developer. Keeprun does not
   *  merge, push or switch branches itself. English, like the issue prompt: it is for the agent. */
  const fixPrompt = $derived.by((): string | null => {
    if (!git) return null;
    const upstream = git.upstream ?? "";
    const branch = git.branch ?? "";
    let state: string;
    let ask: string;
    switch (git.sync) {
      case "diverged":
        state = `${git.ahead} local commits are not on ${upstream}, and ${git.behind} commits on ${upstream} are not here.`;
        ask = `bring ${branch} up to date with ${upstream} the way this project does it (merge or rebase), then push`;
        break;
      case "ahead":
        state = `${git.ahead} commits are not pushed to ${upstream}.`;
        ask = "push them if the work is ready";
        break;
      case "gone":
        state = `${upstream} no longer exists on the remote, often because its pull request merged and the branch was deleted.`;
        ask = `check whether the work is in ${mainBranch}. If it is, switch to ${mainBranch} and pull. If not, push ${branch} again`;
        break;
      case "merged":
        state = `Every commit on ${branch} is already in ${mainBranch}.`;
        ask = `switch to ${mainBranch} and pull, so the app runs the latest code`;
        break;
      default:
        return null;
    }
    return [
      `The Git branch of the app "${app.name}" needs attention. Keeprun runs this app and does not change its repository.`,
      `Folder: ${app.cwd}`,
      `Branch: ${branch}${upstream ? `, upstream: ${upstream}` : ""}`,
      `State: ${state}`,
      git.changed ? `Uncommitted changes: ${git.changed} files.` : "",
      `Look at it with \`git status\` and \`git log --oneline --graph -20 --all\`, then ${ask}. Ask me before anything that rewrites history or could drop work.`,
    ]
      .filter(Boolean)
      .join("\n");
  });
  let promptCopied = $state(false);
  async function copyFixPrompt() {
    if (!fixPrompt) return;
    await navigator.clipboard.writeText(fixPrompt);
    promptCopied = true;
    setTimeout(() => (promptCopied = false), 2000);
  }

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
      // A pull fetches too.
      fetchedAt = Date.now();
      fetchError = "";
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
      if (r.migrations) {
        const task = await desktop.migrateTask(app.tasks);
        if (task) {
          const since = Date.now() - 1000;
          await api.runTask(id, { name: task });
          if (!(await taskSucceeded(id, since))) {
            error = [...parts, t("git.migrateFailed", { task })].join(" ");
            return;
          }
          parts.push(t("git.migrated"));
        } else {
          parts.push(t("git.noMigrateTask"));
        }
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
      {#if label}
        <span
          class="inline-flex items-center gap-1 text-xs tabular-nums {fetchError ? 'text-fg-muted' : toneClass[label.tone]}"
          title={label.hint || undefined}
          role="status"
        >
          {#if git.sync === "merged"}
            <GitMerge size={12} aria-hidden="true" />
          {:else if git.sync === "in_sync"}
            <CircleCheck size={12} aria-hidden="true" />
          {/if}
          {label.text}
        </span>
      {/if}
      {#if git.changed}
        <span class="inline-flex items-center gap-0.5 text-xs tabular-nums text-fg-muted" title={t("git.changedHint", { count: git.changed })}>
          <FilePen size={12} aria-hidden="true" />{git.changed}
        </span>
      {/if}
      {#if canFetch}
        <span class="inline-flex items-center gap-2">
          <button
            class="btn !h-7 !w-7 !px-0"
            onclick={() => refresh()}
            disabled={fetching || pulling}
            title={t("git.refreshHint", { remote })}
            aria-label={t("git.refresh")}
          >
            {#if fetching}
              <LoaderCircle size={13} class="animate-spin motion-reduce:animate-none" aria-hidden="true" />
            {:else}
              <RefreshCw size={13} aria-hidden="true" />
            {/if}
          </button>
          {#if git.sync === "behind"}
            <button
              class="btn !h-7 !px-2 text-xs"
              onclick={pull}
              disabled={pulling || fetching}
              title={t("git.pullHint", { upstream: git.upstream ?? "" })}
            >
              {#if pulling}
                <LoaderCircle size={13} class="animate-spin motion-reduce:animate-none" aria-hidden="true" />
              {:else}
                <Download size={13} aria-hidden="true" />
              {/if}
              {t(isActive(app.state) ? "git.pullRestart" : "git.pull")}
            </button>
          {/if}
          {#if fixPrompt}
            <button class="btn !h-7 !px-2 text-xs" onclick={copyFixPrompt} title={t("overview.copyPromptHint")}>
              <Copy size={13} aria-hidden="true" />
              {promptCopied ? t("common.copied") : t("overview.copyPrompt")}
            </button>
          {/if}
          {#if fetchError}
            <span class="inline-flex items-center gap-1 text-xs text-danger" title={t("git.fetchFailedHint", { error: fetchError })}>
              <CircleAlert size={12} aria-hidden="true" />{t("git.fetchFailed")}
            </span>
          {:else if fetchedAt}
            <span class="text-xs text-fg-muted" title={new Date(fetchedAt).toLocaleString(intlLocale())}>
              {t("git.fetched", { time: timeAgo(fetchedAt, now) })}
            </span>
          {/if}
        </span>
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
