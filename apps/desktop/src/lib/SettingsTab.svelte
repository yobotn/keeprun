<script lang="ts">
  import { untrack } from "svelte";
  import { open } from "@tauri-apps/plugin-dialog";
  import { FolderOpen } from "@lucide/svelte";
  import { api, desktop, isActive, splitTags, type AppView, type RestartPolicy, type Suggestion } from "./api";
  import { t } from "./i18n/index.svelte";

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
      watch_errors: a.watch_errors,
      tags: a.tags.join(", "),
      port: a.env.PORT ?? "",
    };
  }

  const portValid = $derived(form.port.trim() === "" || /^\d{1,5}$/.test(form.port.trim()) && +form.port >= 1 && +form.port <= 65535);

  const dirty = $derived(JSON.stringify(form) !== baseline);
  let error = $state("");
  let saved = $state(false);
  let saving = $state(false);
  let confirmRemove = $state(false);
  let suggestions = $state<Suggestion[]>([]);

  async function browse() {
    const dir = await open({ directory: true, multiple: false, defaultPath: form.cwd, title: t("common.chooseFolder") });
    if (typeof dir === "string") form.cwd = dir;
  }

  async function detect() {
    error = "";
    try {
      suggestions = (await desktop.detect(form.cwd)).suggestions;
      if (!suggestions.length) error = t("settings.nothingDetected");
    } catch (e) {
      error = String(e);
    }
  }

  async function save(e: SubmitEvent) {
    e.preventDefault();
    error = "";
    if (!portValid) {
      error = t("settings.portInvalid");
      return;
    }
    saving = true;
    // Only send env when the port changed, so this form never overwrites Environment edits.
    const port = form.port.trim();
    let env: Record<string, string> | undefined;
    if (port !== (app.env.PORT ?? "")) {
      env = { ...app.env };
      if (port) env.PORT = port;
      else delete env.PORT;
    }
    try {
      const v = await api.update(app.id, {
        ...(env ? { env } : {}),
        name: form.name,
        cwd: form.cwd,
        command: form.command,
        restart: form.restart,
        max_restarts: Number(form.max_restarts) || 0,
        autostart: form.autostart,
        watch_errors: form.watch_errors,
        tags: splitTags(form.tags),
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
        <label class="label" for="s-name">{t("settings.name")}</label>
        <input id="s-name" class="input" bind:value={form.name} required />
        <p class="hint">{@render withCode(t("settings.idHint", { id: "\u0000id\u0000" }), { id: app.id })}</p>
      </div>

      <div>
        <label class="label" for="s-cwd">{t("settings.folder")}</label>
        <div class="flex gap-2">
          <input id="s-cwd" class="input font-mono" bind:value={form.cwd} required />
          <button type="button" class="btn" onclick={browse}><FolderOpen size={14} /> {t("common.browse")}</button>
        </div>
      </div>

      <div>
        <label class="label" for="s-cmd">{t("settings.command")}</label>
        <div class="flex gap-2">
          <input id="s-cmd" class="input font-mono" bind:value={form.command} required spellcheck="false" />
          <button type="button" class="btn" onclick={detect}>{t("settings.detect")}</button>
        </div>
        {#if suggestions.length}
          <div class="mt-2 flex flex-col overflow-hidden rounded-md border border-border">
            {#each suggestions as s}
              <button
                type="button"
                class="flex items-baseline gap-3 px-3 py-1.5 text-start text-[13px] hover:bg-bg-hover"
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
        <p class="hint">{t("settings.commandHint")}</p>
      </div>

      <div>
        <label class="label" for="s-port">{t("settings.port")}</label>
        <input
          id="s-port"
          class="input w-32 tabular-nums {portValid ? '' : '!border-danger'}"
          bind:value={form.port}
          inputmode="numeric"
          maxlength="5"
          placeholder={t("common.none")}
        />
        <p class="hint">
          {@render withCode(t("settings.portHint", { port: "\u0000port\u0000", urls: "\u0000urls\u0000" }), {
            port: "%PORT%",
            urls: "ASPNETCORE_URLS",
          })}
        </p>
      </div>

      <div class="grid grid-cols-2 gap-4">
        <div>
          <label class="label" for="s-restart">{t("settings.restart")}</label>
          <select id="s-restart" class="input" bind:value={form.restart}>
            <option value="on-failure">{t("settings.restartOnFailure")}</option>
            <option value="always">{t("settings.restartAlways")}</option>
            <option value="never">{t("settings.restartNever")}</option>
          </select>
        </div>
        <div>
          <label class="label" for="s-max">{t("settings.maxRestarts")}</label>
          <input id="s-max" class="input" type="number" min="0" bind:value={form.max_restarts} disabled={form.restart === "never"} />
          <p class="hint">{t("settings.maxRestartsHint")}</p>
        </div>
      </div>

      <label class="flex items-start gap-3">
        <input type="checkbox" class="mt-0.5 size-4 accent-[var(--accent)]" bind:checked={form.autostart} />
        <span>
          <span class="text-[13px] font-medium">{t("settings.autostart")}</span>
          <span class="hint block">{t("settings.autostartHint")}</span>
        </span>
      </label>

      <label class="flex items-start gap-3">
        <input type="checkbox" class="mt-0.5 size-4 accent-[var(--accent)]" bind:checked={form.watch_errors} />
        <span>
          <span class="text-[13px] font-medium">{t("settings.watchErrors")}</span>
          <span class="hint block">{t("settings.watchErrorsHint")}</span>
        </span>
      </label>

      <div>
        <label class="label" for="s-tags">{t("settings.tags")}</label>
        <input id="s-tags" class="input" bind:value={form.tags} placeholder={t("settings.tagsPlaceholder")} />
        <p class="hint">{t("settings.tagsHint")}</p>
      </div>

      <div class="mt-4 rounded-lg border border-border p-4">
        <h3 class="text-[13px] font-semibold">{t("settings.removeTitle")}</h3>
        <p class="hint">{t("settings.removeHint")}</p>
        <div class="mt-3 flex gap-2">
          {#if confirmRemove}
            <button type="button" class="btn btn-danger" onclick={remove}>{t("settings.removeName", { name: app.name })}</button>
            <button type="button" class="btn" onclick={() => (confirmRemove = false)}>{t("common.cancel")}</button>
          {:else}
            <button
              type="button"
              class="btn btn-danger"
              disabled={isActive(app.state)}
              title={isActive(app.state) ? t("settings.stopFirst") : ""}
              onclick={() => (confirmRemove = true)}>{t("common.remove")}</button
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
        <p class="flex-1 text-sm text-fg-muted">{t("common.saved")}{isActive(app.state) ? ` ${t("settings.restartToApply")}` : ""}</p>
      {:else}
        <p class="flex-1 text-sm text-fg-muted">{t("common.unsaved")}</p>
      {/if}
      {#if dirty}
        <button type="button" class="btn" onclick={() => (form = fresh(app), baseline = JSON.stringify(form))}>{t("common.discard")}</button>
        <button type="submit" class="btn btn-primary" disabled={saving}>{t(saving ? "common.saving" : "common.save")}</button>
      {/if}
    </div>
  {/if}
</form>

{#snippet withCode(text: string, codes: Record<string, string>)}
  {#each text.split("\u0000") as part, i}{#if i % 2 === 1}<span class="font-mono">{codes[part] ?? part}</span>{:else}{part}{/if}{/each}
{/snippet}
