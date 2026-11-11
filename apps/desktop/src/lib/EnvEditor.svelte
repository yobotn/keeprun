<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { Eye, EyeOff, FileInput, Lock, LockOpen, Plus, Trash2 } from "@lucide/svelte";
  import { api, desktop, isActive, type AppView } from "./api";
  import { t } from "./i18n/index.svelte";

  let { app, onsaved }: { app: AppView; onsaved: () => void } = $props();

  interface Row {
    key: string;
    value: string;
    secret: boolean;
    reveal: boolean;
  }

  function fromApp(a: AppView): Row[] {
    return Object.entries(a.env).map(([key, value]) => ({ key, value, secret: a.secrets.includes(key), reveal: false }));
  }

  // Reset only when switching apps, so polling does not overwrite edits in progress.
  let rows = $state<Row[]>([]);
  let loadedFor = "";
  let baseline = "";
  $effect(() => {
    if (app.id !== loadedFor) {
      loadedFor = app.id;
      rows = fromApp(app);
      baseline = snapshot(rows);
    }
  });

  function snapshot(r: Row[]) {
    return JSON.stringify(r.map(({ key, value, secret }) => [key, value, secret]));
  }

  const dirty = $derived(snapshot(rows) !== baseline);
  let error = $state("");
  let saved = $state(false);
  let saving = $state(false);

  const duplicate = $derived.by(() => {
    const seen = new Set<string>();
    for (const r of rows) {
      const k = r.key.trim();
      if (k && seen.has(k)) return k;
      seen.add(k);
    }
    return null;
  });

  function addRow() {
    rows.push({ key: "", value: "", secret: false, reveal: true });
    queueMicrotask(() => document.querySelector<HTMLInputElement>("[data-env-key]:last-of-type")?.focus());
  }

  async function importFile() {
    error = "";
    const file = await open({ multiple: false, title: t("env.importTitle"), defaultPath: app.cwd });
    if (typeof file !== "string") return;
    try {
      const vars = await desktop.readEnvFile(file);
      for (const [key, value] of Object.entries(vars)) {
        const existing = rows.find((r) => r.key === key);
        if (existing) existing.value = value;
        else rows.push({ key, value, secret: await desktop.looksSecret(key), reveal: false });
      }
    } catch (e) {
      error = String(e);
    }
  }

  async function save() {
    error = "";
    if (duplicate) {
      error = t("env.duplicate", { name: duplicate });
      return;
    }
    const clean = rows.filter((r) => r.key.trim());
    const env = Object.fromEntries(clean.map((r) => [r.key.trim(), r.value]));
    const secrets = clean.filter((r) => r.secret).map((r) => r.key.trim());
    saving = true;
    try {
      const v = await api.update(app.id, { env, secrets });
      rows = fromApp(v);
      baseline = snapshot(rows);
      saved = true;
      setTimeout(() => (saved = false), 2500);
      onsaved();
    } catch (e) {
      error = (e as Error).message;
    } finally {
      saving = false;
    }
  }

  function discard() {
    rows = fromApp(app);
    baseline = snapshot(rows);
    error = "";
  }
</script>

<div class="flex min-h-0 flex-1 flex-col">
  <div class="flex items-center gap-2 border-b border-border px-6 py-2.5">
    <p class="flex-1 text-xs text-fg-muted">{t("env.intro")}</p>
    <button class="btn" onclick={importFile}><FileInput size={14} /> {t("env.import")}</button>
    <button class="btn" onclick={addRow}><Plus size={14} /> {t("env.add")}</button>
  </div>

  <div class="flex-1 overflow-auto px-6 py-3">
    {#if rows.length === 0}
      <p class="py-6 text-sm text-fg-muted">{t("env.none")}</p>
    {:else}
      <div class="grid grid-cols-[minmax(140px,1fr)_2fr_auto] items-center gap-x-2 gap-y-1.5">
        <span class="text-xs font-medium text-fg-muted">{t("env.name")}</span>
        <span class="text-xs font-medium text-fg-muted">{t("env.value")}</span>
        <span></span>
        {#each rows as row, i}
          <input
            data-env-key
            class="input font-mono"
            bind:value={row.key}
            placeholder="NAME"
            spellcheck="false"
            aria-label={t("env.nameLabel")}
          />
          <input
            class="input font-mono"
            type={row.secret && !row.reveal ? "password" : "text"}
            bind:value={row.value}
            spellcheck="false"
            aria-label={t("env.valueLabel", { name: row.key || t("env.newVariable") })}
          />
          <div class="flex items-center">
            {#if row.secret}
              <button class="btn-icon" onclick={() => (row.reveal = !row.reveal)} title={t(row.reveal ? "env.hide" : "env.show")}>
                {#if row.reveal}<EyeOff size={14} />{:else}<Eye size={14} />{/if}
              </button>
            {/if}
            <button
              class="btn-icon {row.secret ? '!text-accent' : ''}"
              onclick={() => (row.secret = !row.secret)}
              title={t(row.secret ? "env.secret" : "env.plain")}
            >
              {#if row.secret}<Lock size={14} />{:else}<LockOpen size={14} />{/if}
            </button>
            <button class="btn-icon hover:!text-danger" onclick={() => rows.splice(i, 1)} title={t("env.removeVar")}>
              <Trash2 size={14} />
            </button>
          </div>
        {/each}
      </div>
    {/if}
  </div>

  {#if error || dirty || saved}
    <div class="flex items-center gap-3 border-t border-border px-6 py-3">
      {#if error}
        <p class="alert flex-1" role="alert">{error}</p>
      {:else if saved}
        <p class="flex-1 text-sm text-fg-muted">
          {t("common.saved")}{isActive(app.state) ? ` ${t("env.restartToApply")}` : ""}
        </p>
      {:else}
        <p class="flex-1 text-sm text-fg-muted">{t("common.unsaved")}</p>
      {/if}
      {#if dirty}
        <button class="btn" onclick={discard}>{t("common.discard")}</button>
        <button class="btn btn-primary" onclick={save} disabled={saving}>{t(saving ? "common.saving" : "common.save")}</button>
      {/if}
    </div>
  {/if}
</div>
