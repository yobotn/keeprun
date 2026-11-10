<script lang="ts">
  import { untrack } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { FolderOpen } from "@lucide/svelte";
  import { api, desktop, isActive, type AppView, type RestartPolicy, type Suggestion } from "./api";

  let { app, onsaved, onremoved }: { app: AppView; onsaved: () => void; onremoved: () => void } = $props();

  let form = $state(fresh(untrack(() => app)));
  let loadedFor = "";
  let baseline = "";
  $effect(() => {
    if (app.id !== loadedFor) {
      loadedFor = app.id;
      form = fresh(app);
      baseline = JSON.stringify(form);
      suggestions = [];
    }
  });

  function fresh(a: AppView) {
    return {
      name: a.name,
      cwd: a.cwd,
      command: a.command,
      restart: a.restart as RestartPolicy,
      max_restarts: a.max_restarts,
      autostart: a.autostart,
      tags: a.tags.join(", "),
    };
  }

  const dirty = $derived(JSON.stringify(form) !== baseline);
  let error = $state("");
  let saved = $state(false);
  let saving = $state(false);
  let confirmRemove = $state(false);
  let suggestions = $state<Suggestion[]>([]);

  async function browse() {
    const dir = await open({ directory: true, multiple: false, defaultPath: form.cwd, title: "Choose the app folder" });
    if (typeof dir === "string") form.cwd = dir;
  }

  async function detect() {
    error = "";
    try {
      suggestions = (await desktop.detect(form.cwd)).suggestions;
      if (!suggestions.length) error = "Nothing to run was detected in this folder.";
    } catch (e) {
      error = String(e);
    }
  }

  async function save(e: SubmitEvent) {
    e.preventDefault();
    error = "";
    saving = true;
    try {
      const v = await api.update(app.id, {
        name: form.name,
        cwd: form.cwd,
        command: form.command,
        restart: form.restart,
        max_restarts: Number(form.max_restarts) || 0,
        autostart: form.autostart,
        tags: form.tags.split(",").map((t) => t.trim()).filter(Boolean),
      });
      form = fresh(v);
      baseline = JSON.stringify(form);
      saved = true;
      setTimeout(() => (saved = false), 2500);
      onsaved();
    } catch (err) {
      error = (err as Error).message;
    } finally {
      saving = false;
    }
  }

  async function remove() {
    error = "";
    try {
      await api.remove(app.id);
      onremoved();
    } catch (e) {
      error = (e as Error).message;
      confirmRemove = false;
    }
  }
</script>

<form onsubmit={save} class="flex min-h-0 flex-1 flex-col">
  <div class="flex-1 overflow-auto px-6 py-5">
    <div class="flex max-w-2xl flex-col gap-5">
      <div>
        <label class="label" for="s-name">Name</label>
        <input id="s-name" class="input" bind:value={form.name} required />
        <p class="hint">The app id stays <span class="font-mono">{app.id}</span> so scripts keep working.</p>
      </div>

      <div>
        <label class="label" for="s-cwd">Folder</label>
        <div class="flex gap-2">
          <input id="s-cwd" class="input font-mono" bind:value={form.cwd} required />
          <button type="button" class="btn" onclick={browse}><FolderOpen size={14} /> Browse</button>
        </div>
      </div>

      <div>
        <label class="label" for="s-cmd">Command</label>
        <div class="flex gap-2">
          <input id="s-cmd" class="input font-mono" bind:value={form.command} required spellcheck="false" />
          <button type="button" class="btn" onclick={detect}>Detect</button>
        </div>
        {#if suggestions.length}
          <div class="mt-2 flex flex-col overflow-hidden rounded-md border border-border">
            {#each suggestions as s}
              <button
                type="button"
                class="flex items-baseline gap-3 px-3 py-1.5 text-left text-[13px] hover:bg-bg-hover"
                onclick={() => {
                  form.command = s.command;
                  suggestions = [];
                }}
              >
                <span class="font-medium">{s.label}</span>
                {#if s.detail}<span class="truncate font-mono text-xs text-fg-muted">{s.detail}</span>{/if}
              </button>
            {/each}
          </div>
        {/if}
        <p class="hint">Runs through cmd.exe in the folder. Full paths work under any account.</p>
      </div>

      <div class="grid grid-cols-2 gap-4">
        <div>
          <label class="label" for="s-restart">Restart on exit</label>
          <select id="s-restart" class="input" bind:value={form.restart}>
            <option value="on-failure">On failure (exit code not 0)</option>
            <option value="always">Always</option>
            <option value="never">Never</option>
          </select>
        </div>
        <div>
          <label class="label" for="s-max">Max restarts in a row</label>
          <input id="s-max" class="input" type="number" min="0" bind:value={form.max_restarts} disabled={form.restart === "never"} />
          <p class="hint">Then the app is marked Failed. 0 means no limit. Waits 1s, 2s, 4s, up to 60s.</p>
        </div>
      </div>

      <label class="flex items-start gap-3">
        <input type="checkbox" class="mt-0.5 size-4 accent-[var(--accent)]" bind:checked={form.autostart} />
        <span>
          <span class="text-[13px] font-medium">Start with the agent</span>
          <span class="hint block">After a reboot or agent restart, start the app again if it was running. A stopped app stays stopped.</span>
        </span>
      </label>

      <div>
        <label class="label" for="s-tags">Tags</label>
        <input id="s-tags" class="input" bind:value={form.tags} placeholder="finance, internal" />
        <p class="hint">Comma separated. Apps are grouped by their first tag.</p>
      </div>

      <div class="mt-4 rounded-lg border border-border p-4">
        <h3 class="text-[13px] font-semibold">Remove app</h3>
        <p class="hint">Removes the app and its events. Logs and the folder are kept.</p>
        <div class="mt-3 flex gap-2">
          {#if confirmRemove}
            <button type="button" class="btn btn-danger" onclick={remove}>Remove {app.name}</button>
            <button type="button" class="btn" onclick={() => (confirmRemove = false)}>Cancel</button>
          {:else}
            <button
              type="button"
              class="btn btn-danger"
              disabled={isActive(app.state)}
              title={isActive(app.state) ? "Stop the app first" : ""}
              onclick={() => (confirmRemove = true)}>Remove</button
            >
          {/if}
        </div>
      </div>
    </div>
  </div>

  {#if error || dirty || saved}
    <div class="flex items-center gap-3 border-t border-border px-6 py-3">
      {#if error}
        <p class="alert flex-1" role="alert">{error}</p>
      {:else if saved}
        <p class="flex-1 text-sm text-fg-muted">Saved.{isActive(app.state) ? " Restart to apply folder or command changes." : ""}</p>
      {:else}
        <p class="flex-1 text-sm text-fg-muted">Unsaved changes</p>
      {/if}
      {#if dirty}
        <button type="button" class="btn" onclick={() => (form = fresh(app), baseline = JSON.stringify(form))}>Discard</button>
        <button type="submit" class="btn btn-primary" disabled={saving}>{saving ? "Saving" : "Save"}</button>
      {/if}
    </div>
  {/if}
</form>
