<script lang="ts">
  import { Pencil, Play, Plus, Square, Trash2 } from "@lucide/svelte";
  import { api, formatDuration, formatTime, type AppView, type TaskSpec } from "./api";
  import LogPanel from "./LogPanel.svelte";

  let { app, now, onchanged }: { app: AppView; now: number; onchanged: () => void } = $props();

  let error = $state("");
  let busy = $state(false);
  let oneOff = $state("");
  // Row being edited: index into app.tasks, or -1 for a new task.
  let editing = $state<number | null>(null);
  let draft = $state<TaskSpec>({ name: "", command: "" });

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

  function removeTask(i: number) {
    act(() => api.update(app.id, { tasks: app.tasks.filter((_, j) => j !== i) }));
  }

  const status = $derived.by(() => {
    if (!run) return "";
    if (run.running) return `Running for ${formatDuration(now - run.started_at)}`;
    const took = run.finished_at ? ` in ${formatDuration(run.finished_at - run.started_at)}` : "";
    if (run.exit_code === 0) return `Finished${took}`;
    if (run.exit_code === null) return `Stopped${took}`;
    return `Failed with exit code ${run.exit_code}${took}`;
  });
</script>

<div class="flex min-h-0 flex-1 flex-col">
  <div class="max-h-[50%] shrink-0 overflow-auto border-b border-border px-6 py-4">
    <p class="hint !mt-0 mb-3">
      Tasks are one-off commands, such as migrations or installs. They run in the app folder with the app
      environment, and do not start or stop the app.
    </p>

    {#if app.tasks.length || editing === -1}
      <ul class="flex flex-col overflow-hidden rounded-md border border-border">
        {#each app.tasks as t, i (t.name)}
          {#if editing === i}
            {@render editor()}
          {:else}
            <li class="flex items-center gap-3 border-b border-border px-3 py-1.5 last:border-0 hover:bg-bg-hover">
              <span class="w-40 shrink-0 truncate text-[13px] font-medium" title={t.name}>{t.name}</span>
              <span class="selectable min-w-0 flex-1 truncate font-mono text-xs text-fg-muted" title={t.command}>{t.command}</span>
              <button class="btn-icon" onclick={() => edit(i)} title="Edit {t.name}" disabled={busy}><Pencil size={14} /></button>
              <button class="btn-icon" onclick={() => removeTask(i)} title="Remove {t.name}" disabled={busy}><Trash2 size={14} /></button>
              <button class="btn !h-7" onclick={() => runSaved(t.name)} disabled={busy || running} title={running ? "A task is running" : `Run ${t.name}`}>
                <Play size={12} /> Run
              </button>
            </li>
          {/if}
        {/each}
        {#if editing === -1}{@render editor()}{/if}
      </ul>
    {:else}
      <p class="text-[13px] text-fg-muted">No saved tasks.</p>
    {/if}

    <div class="mt-3 flex items-center gap-2">
      {#if editing === null}
        <button class="btn" onclick={() => edit(-1)} disabled={busy}><Plus size={14} /> Add task</button>
      {/if}
      <form class="flex flex-1 gap-2" onsubmit={runOneOff}>
        <input
          class="input font-mono"
          bind:value={oneOff}
          placeholder="Run a command once, e.g. npm run seed"
          spellcheck="false"
          aria-label="Command to run once"
        />
        <button class="btn" type="submit" disabled={busy || running || !oneOff.trim()}><Play size={12} /> Run</button>
      </form>
    </div>

    {#if error}<p class="selectable alert mt-3" role="alert">{error}</p>{/if}
  </div>

  {#if run}
    <div class="flex items-center gap-3 border-b border-border px-6 py-2 text-[13px]">
      <span class="min-w-0 truncate font-medium" title={run.command}>{run.name ?? run.command}</span>
      <span
        class={running ? "text-accent" : run.exit_code === 0 ? "text-fg-muted" : "text-danger"}
        title="Started {formatTime(run.started_at, true)}">{status}</span
      >
      <div class="flex-1"></div>
      {#if running}
        <button class="btn !h-7" onclick={stopTask} disabled={busy}><Square size={12} /> Stop task</button>
      {/if}
    </div>
    {#key run.started_at}
      <LogPanel appId={app.id} taskSince={run.started_at} />
    {/key}
  {:else}
    <p class="px-6 py-4 text-[13px] text-fg-muted">Task output shows here.</p>
  {/if}
</div>

{#snippet editor()}
  <li class="border-b border-border bg-bg-subtle px-3 py-2 last:border-0">
    <form class="flex items-center gap-2" onsubmit={saveTask}>
      <!-- svelte-ignore a11y_autofocus -->
      <input class="input w-40 shrink-0" bind:value={draft.name} placeholder="migrate" required aria-label="Task name" autofocus />
      <input
        class="input flex-1 font-mono"
        bind:value={draft.command}
        placeholder="npm run db:migrate"
        required
        spellcheck="false"
        aria-label="Task command"
      />
      <button type="button" class="btn" onclick={() => (editing = null)}>Cancel</button>
      <button type="submit" class="btn btn-primary" disabled={busy}>Save</button>
    </form>
  </li>
{/snippet}
