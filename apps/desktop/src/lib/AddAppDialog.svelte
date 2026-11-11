<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { FolderOpen, TriangleAlert, X } from "@lucide/svelte";
  import { api, desktop, type AppView, type Detection } from "./api";
  import { t } from "./i18n/index.svelte";

  // `folder` comes from the Explorer menu: skip the picker and detect it right away.
  let { folder, onclose, onadded }: { folder?: string; onclose: () => void; onadded: (app: AppView) => void } = $props();

  let dialog: HTMLDialogElement;
  let cwd = $state("");
  let detection = $state<Detection | null>(null);
  let name = $state("");
  let command = $state("");
  let port = $state("");
  let custom = $state(false);
  let importEnv = $state(true);
  let startNow = $state(true);
  let error = $state("");
  let busy = $state(false);

  $effect(() => {
    dialog.showModal();
    // Open the folder picker right away: it is the first step for almost everyone.
    if (folder) useFolder(folder);
    else pickFolder();
  });

  async function pickFolder() {
    const dir = await open({ directory: true, multiple: false, title: t("common.chooseFolder") });
    if (typeof dir === "string") await useFolder(dir);
  }

  async function useFolder(dir: string) {
    cwd = dir;
    error = "";
    detection = null;
    try {
      const d = await desktop.detect(dir);
      detection = d;
      name = d.name;
      port = d.dotenv.PORT ?? (d.port ? String(d.port) : "");
      command = d.suggestions[0]?.command ?? "";
      custom = d.suggestions.length === 0;
    } catch (e) {
      error = String(e);
    }
  }

  async function submit(e: SubmitEvent) {
    e.preventDefault();
    if (!cwd) return pickFolder();
    error = "";
    busy = true;
    try {
      const env: Record<string, string> = importEnv && detection ? { ...detection.dotenv } : {};
      if (port.trim()) env.PORT = port.trim();
      const secrets: string[] = [];
      for (const k of Object.keys(env)) if (await desktop.looksSecret(k)) secrets.push(k);
      let app = await api.add({
        name,
        cwd,
        command,
        env,
        secrets,
        tags: [],
        autostart: true,
        restart: "on-failure",
        max_restarts: 5,
        // One-off commands (install, migrate, build) are saved, never run on start.
        tasks: (detection?.tasks ?? []).map((t) => ({ name: t.name || t.label, command: t.command })),
      });
      if (startNow) {
        try {
          app = await api.start(app.id);
        } catch {
          // The app was saved. Its start error shows in Overview and Logs.
        }
      }
      onadded(app);
    } catch (err) {
      error = (err as Error).message;
    } finally {
      busy = false;
    }
  }

  const kindLabel = $derived({ node: "Node.js", dotnet: ".NET", python: "Python", custom: t("add.kindCustom") });
</script>

<dialog bind:this={dialog} {onclose} class="dialog w-[580px] max-w-[calc(100vw-32px)]">
  <form onsubmit={submit} class="flex flex-col gap-4 p-5">
    <div class="flex items-center justify-between">
      <h2 class="text-base font-semibold">{t("add.title")}</h2>
      <button type="button" class="btn-icon" onclick={() => dialog.close()} title={t("common.close")}><X size={16} /></button>
    </div>

    <div>
      <label class="label" for="a-cwd">{t("add.folder")}</label>
      <div class="flex gap-2">
        <input
          id="a-cwd"
          class="input font-mono"
          bind:value={cwd}
          placeholder="D:\apps\hr-portal"
          onchange={() => cwd && useFolder(cwd)}
        />
        <button type="button" class="btn" onclick={pickFolder}><FolderOpen size={14} /> {t("common.browse")}</button>
      </div>
    </div>

    {#if detection}
      <p class="-mt-1 text-xs text-fg-muted">
        {detection.port
          ? t("add.detectedPort", { kind: kindLabel[detection.kind], port: detection.port })
          : t("add.detected", { kind: kindLabel[detection.kind] })}
      </p>

      {#each detection.warnings as w}
        <p class="alert alert-warn flex items-start gap-2"><TriangleAlert size={14} class="mt-0.5 shrink-0" />{w}</p>
      {/each}

      <div class="flex gap-3">
        <div class="flex-1">
          <label class="label" for="a-name">{t("add.name")}</label>
          <input id="a-name" class="input" bind:value={name} required />
        </div>
        <div class="w-28">
          <label class="label" for="a-port">{t("add.port")}</label>
          <input
            id="a-port"
            class="input tabular-nums"
            bind:value={port}
            inputmode="numeric"
            pattern="\d{'{'}1,5{'}'}"
            maxlength="5"
            placeholder={t("common.none")}
            title={t("add.portHint")}
          />
        </div>
      </div>

      <fieldset>
        <legend class="label">{t("add.command")}</legend>
        <div class="flex max-h-56 flex-col overflow-auto rounded-md border border-border">
          {#each detection.suggestions as s}
            <label class="flex cursor-default items-start gap-3 border-b border-border px-3 py-2 last:border-0 hover:bg-bg-hover">
              <input
                type="radio"
                name="cmd"
                class="mt-1 accent-[var(--accent)]"
                checked={!custom && command === s.command}
                onchange={() => {
                  command = s.command;
                  custom = false;
                }}
              />
              <span class="min-w-0">
                <span class="block text-[13px] font-medium">{s.label}</span>
                {#if s.detail}<span class="block truncate font-mono text-xs text-fg-muted">{s.detail}</span>{/if}
              </span>
            </label>
          {/each}
          <label class="flex cursor-default items-center gap-3 px-3 py-2 hover:bg-bg-hover">
            <input type="radio" name="cmd" class="accent-[var(--accent)]" checked={custom} onchange={() => (custom = true)} />
            <span class="text-[13px] font-medium">{t("add.customCommand")}</span>
          </label>
        </div>
        {#if custom}
          <input class="input mt-2 font-mono" bind:value={command} required placeholder="node server.js" spellcheck="false" />
        {:else}
          <p class="hint selectable truncate font-mono">{command}</p>
        {/if}
      </fieldset>

      {#if detection.tasks.length}
        <p class="hint !mt-0">
          {t("add.tasksFound", {
            count: detection.tasks.length,
            names: detection.tasks.map((task) => task.name || task.label).join(", "),
          })}
        </p>
      {/if}

      {#if Object.keys(detection.dotenv).length}
        <label class="flex items-center gap-2 text-[13px]">
          <input type="checkbox" class="accent-[var(--accent)]" bind:checked={importEnv} />
          {t("add.importEnv", { count: Object.keys(detection.dotenv).length })}
        </label>
      {/if}
      <label class="flex items-center gap-2 text-[13px]">
        <input type="checkbox" class="accent-[var(--accent)]" bind:checked={startNow} />
        {t("add.startNow")}
      </label>
    {:else if !error}
      <p class="py-4 text-center text-sm text-fg-muted">{t("add.chooseFolder")}</p>
    {/if}

    {#if error}<p class="alert" role="alert">{error}</p>{/if}

    <div class="flex justify-end gap-2">
      <button type="button" class="btn" onclick={() => dialog.close()}>{t("common.cancel")}</button>
      <button type="submit" class="btn btn-primary" disabled={busy || !detection || !command.trim()}>
        {t(busy ? "common.saving" : startNow ? "add.saveAndStart" : "common.save")}
      </button>
    </div>
  </form>
</dialog>
