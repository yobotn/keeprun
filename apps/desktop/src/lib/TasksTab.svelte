<script lang="ts">
  import { Pencil, Play, Plus, Square, Trash2 } from "@lucide/svelte";
  import { api, desktop, formatDuration, formatTime, type AppView, type Suggestion, type TaskSpec } from "./api";
  import { t } from "./i18n/index.svelte";
  import LogPanel from "./LogPanel.svelte";

  let { app, now, onchanged }: { app: AppView; now: number; onchanged: () => void } = $props();

  let error = $state("");
  let busy = $state(false);
  let oneOff = $state("");
  // Row being edited: index into app.tasks, or -1 for a new task.
  let editing = $state<number | null>(null);
  let draft = $state<TaskSpec>({ name: "", command: "" });

  // Job scripts found in the app folder (build, test, migrate, install), so nobody has to guess.
  let found = $state<Suggestion[]>([]);
  // The app list is polled and each poll is a new object. Rescan only when the folder changes.
  const folder = $derived(app.cwd);
  $effect(() => {
    const cwd = folder;
    let stale = false;
    desktop
      .detect(cwd)
      .then((d) => {
        // Only one-off jobs. Server scripts (start, dev, serve) are the app's own command.
        if (!stale) found = d.tasks;
      })
      .catch(() => {
        if (!stale) found = [];
      });
    return () => {
      stale = true;
    };
  });
  const available = $derived(found.filter((s) => !app.tasks.some((task) => task.command === s.command)));

  const run = $derived(app.task);
  const running = $derived(run?.running ?? false);

  async function act(fn: () => Promise<unknown>) {
    busy = true;
    error = "";
    try {
      await fn();
    } catch (e) {
      error = (e as Error).message;
    } finally {
      busy = false;
      onchanged();
    }
  }

  const runSaved = (name: string) => act(() => api.runTask(app.id, { name }));
  const stopTask = () => act(() => api.stopTask(app.id));

  function runOneOff(e: SubmitEvent) {
    e.preventDefault();
    const command = oneOff.trim();
    if (!command) return;
    act(async () => {
      await api.runTask(app.id, { command });
      oneOff = "";
    });
  }

  function edit(i: number) {
    editing = i;
    draft = i >= 0 ? { ...app.tasks[i] } : { name: "", command: oneOff.trim() };
    error = "";
  }

  function saveTask(e: SubmitEvent) {
    e.preventDefault();
    const tasks = [...app.tasks];
    if (editing === -1) tasks.push({ ...draft });
    else if (editing !== null) tasks[editing] = { ...draft };
    act(async () => {
      await api.update(app.id, { tasks });
      editing = null;
    });
  }

  const runFound = (s: Suggestion) => act(() => api.runTask(app.id, { command: s.command }));

  function saveFound(s: Suggestion) {
    let name = s.name;
    for (let n = 2; app.tasks.some((task) => task.name === name); n++) name = `${s.name} ${n}`;
    act(() => api.update(app.id, { tasks: [...app.tasks, { name, command: s.command }] }));
  }

  function removeTask(i: number) {
    act(() => api.update(app.id, { tasks: app.tasks.filter((_, j) => j !== i) }));
  }

  const status = $derived.by(() => {
    if (!run) return "";
    if (run.running) return t("tasks.runningFor", { duration: formatDuration(now - run.started_at) });
    const duration = run.finished_at ? formatDuration(run.finished_at - run.started_at) : null;
    if (run.exit_code === 0) return duration ? t("tasks.finishedIn", { duration }) : t("tasks.finished");
    if (run.exit_code === null) return duration ? t("tasks.stoppedIn", { duration }) : t("tasks.stopped");
    const code = run.exit_code;
    return duration ? t("tasks.failedIn", { code, duration }) : t("tasks.failed", { code });
  });
</script>

<div class="flex min-h-0 flex-1 flex-col">
  <div class="max-h-[50%] shrink-0 overflow-auto border-b border-border px-6 py-4">
    <p class="hint !mt-0 mb-3">{t("tasks.intro")}</p>

    {#if app.tasks.length || editing === -1}
      <ul class="flex flex-col overflow-hidden rounded-md border border-border">
        {#each app.tasks as task, i (task.name)}
          {#if editing === i}
            {@render editor()}
          {:else}
            <li class="flex items-center gap-3 border-b border-border px-3 py-1.5 last:border-0 hover:bg-bg-hover">
              <span class="w-40 shrink-0 truncate text-[13px] font-medium" title={task.name}>{task.name}</span>
              <span class="selectable min-w-0 flex-1 truncate font-mono text-xs text-fg-muted" title={task.command}>{task.command}</span>
              <button class="btn-icon" onclick={() => edit(i)} title={t("tasks.edit", { name: task.name })} disabled={busy}><Pencil size={14} /></button>
              <button class="btn-icon" onclick={() => removeTask(i)} title={t("tasks.remove", { name: task.name })} disabled={busy}><Trash2 size={14} /></button>
              <button class="btn !h-7" onclick={() => runSaved(task.name)} disabled={busy || running} title={running ? t("tasks.aTaskRunning") : t("tasks.runName", { name: task.name })}>
                <Play size={12} class="flip-rtl" /> {t("common.run")}
              </button>
            </li>
          {/if}
        {/each}
        {#if editing === -1}{@render editor()}{/if}
      </ul>
    {:else}
      <p class="text-[13px] text-fg-muted">{t("tasks.none")}</p>
    {/if}

    {#if available.length}
      <h3 class="mt-4 mb-2 text-xs font-medium text-fg-muted">{t("tasks.available")}</h3>
      <ul class="flex flex-col overflow-hidden rounded-md border border-border">
        {#each available as s (s.command)}
          <li class="flex items-center gap-3 border-b border-border px-3 py-1.5 last:border-0 hover:bg-bg-hover">
            <span class="w-40 shrink-0 truncate font-mono text-[13px]" title={s.label}>{s.name}</span>
            <span class="selectable min-w-0 flex-1 truncate font-mono text-xs text-fg-muted" title={s.detail ?? s.label}>{s.detail ?? s.label}</span>
            <button class="btn-icon" onclick={() => saveFound(s)} title={t("tasks.saveName", { name: s.name })} disabled={busy}><Plus size={14} /></button>
            <button class="btn !h-7" onclick={() => runFound(s)} disabled={busy || running} title={running ? t("tasks.aTaskRunning") : t("tasks.runName", { name: s.label })}>
              <Play size={12} class="flip-rtl" /> {t("common.run")}
            </button>
          </li>
        {/each}
      </ul>
    {/if}

    <div class="mt-3 flex items-center gap-2">
      {#if editing === null}
        <button class="btn" onclick={() => edit(-1)} disabled={busy}><Plus size={14} /> {t("tasks.add")}</button>
      {/if}
      <form class="flex flex-1 gap-2" onsubmit={runOneOff}>
        <input
          class="input font-mono"
          bind:value={oneOff}
          placeholder={t("tasks.oneOffPlaceholder")}
          spellcheck="false"
          aria-label={t("tasks.oneOffLabel")}
        />
        <button class="btn" type="submit" disabled={busy || running || !oneOff.trim()}><Play size={12} class="flip-rtl" /> {t("common.run")}</button>
      </form>
    </div>

    {#if error}<p class="selectable alert mt-3" role="alert">{error}</p>{/if}
  </div>

  {#if run}
    <div class="flex items-center gap-3 border-b border-border px-6 py-2 text-[13px]">
      <span class="min-w-0 truncate font-medium" title={run.command}>{run.name ?? run.command}</span>
      <span
        class={running ? "text-accent" : run.exit_code === 0 ? "text-fg-muted" : "text-danger"}
        title={t("tasks.started", { time: formatTime(run.started_at, true) })}>{status}</span
      >
      <div class="flex-1"></div>
      {#if running}
        <button class="btn !h-7" onclick={stopTask} disabled={busy}><Square size={12} /> {t("tasks.stop")}</button>
      {/if}
    </div>
    {#key run.started_at}
      <LogPanel appId={app.id} cwd={app.cwd} taskSince={run.started_at} />
    {/key}
  {:else}
    <p class="px-6 py-4 text-[13px] text-fg-muted">{t("tasks.outputHere")}</p>
  {/if}
</div>

{#snippet editor()}
  <li class="border-b border-border bg-bg-subtle px-3 py-2 last:border-0">
    <form class="flex items-center gap-2" onsubmit={saveTask}>
      <!-- svelte-ignore a11y_autofocus -->
      <input class="input w-40 shrink-0" bind:value={draft.name} placeholder="migrate" required aria-label={t("tasks.nameLabel")} autofocus />
      <input
        class="input flex-1 font-mono"
        bind:value={draft.command}
        placeholder="npm run db:migrate"
        required
        spellcheck="false"
        aria-label={t("tasks.commandLabel")}
      />
      <button type="button" class="btn" onclick={() => (editing = null)}>{t("common.cancel")}</button>
      <button type="submit" class="btn btn-primary" disabled={busy}>{t("common.save")}</button>
    </form>
  </li>
{/snippet}
