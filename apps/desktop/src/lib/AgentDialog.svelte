<script lang="ts">
  import { X } from "@lucide/svelte";
  import { connection, desktop, type Connection } from "./api";
  import { checkForUpdates, updates } from "./updates.svelte";
  import { t } from "./i18n/index.svelte";

  let {
    conn,
    onclose,
    onchanged,
    onupdate,
  }: { conn: Connection | null; onclose: () => void; onchanged: () => void; onupdate: () => void } = $props();

  let dialog: HTMLDialogElement;
  const currentPort = $derived(conn ? Number(new URL(conn.baseUrl).port) : 0);
  let port = $state("");
  let busy = $state(false);
  let error = $state("");
  let message = $state("");

  $effect(() => {
    dialog.showModal();
  });

  $effect(() => {
    if (!port && currentPort) port = String(currentPort);
  });

  const valid = $derived(/^\d+$/.test(port) && Number(port) >= 1 && Number(port) <= 65535);

  async function change(e: SubmitEvent) {
    e.preventDefault();
    if (!valid || Number(port) === currentPort) return;
    busy = true;
    error = "";
    message = "";
    try {
      message = await desktop.setAgentPort(Number(port));
      await connection(true);
      onchanged();
    } catch (err) {
      error = String(err);
    } finally {
      busy = false;
    }
  }
</script>

<dialog bind:this={dialog} {onclose} class="dialog w-[520px] max-w-[calc(100vw-32px)]">
  <div class="flex flex-col gap-4 p-5">
    <div class="flex items-center justify-between">
      <h2 class="text-base font-semibold">{t("agent.title")}</h2>
      <button type="button" class="btn-icon" onclick={() => dialog.close()} title={t("common.close")}><X size={16} /></button>
    </div>

    {#if conn}
      <dl class="selectable grid grid-cols-[minmax(110px,auto)_1fr] gap-x-4 gap-y-1.5 text-[13px]">
        <dt class="text-fg-muted">{t("agent.environment")}</dt>
        <dd>{conn.env}</dd>
        <dt class="text-fg-muted">{t("agent.address")}</dt>
        <dd class="font-mono">{conn.baseUrl}</dd>
        <dt class="text-fg-muted">{t("agent.dataFolder")}</dt>
        <dd class="truncate font-mono" title={conn.dataDir}>{conn.dataDir}</dd>
        <dt class="text-fg-muted">{t("agent.version")}</dt>
        <dd class="flex items-center gap-3">
          {conn.version}
          {#if updates.available}
            <button type="button" class="text-accent hover:underline" onclick={onupdate}>{t("agent.updateTo", { version: updates.available.version })}</button>
          {:else if updates.policy && !updates.policy.enabled}
            <span class="text-fg-muted">{updates.policy.reason}</span>
          {:else}
            <button type="button" class="text-accent hover:underline disabled:opacity-60" disabled={updates.checking} onclick={() => checkForUpdates(true)}>
              {t(updates.checking ? "agent.checking" : "agent.check")}
            </button>
            {#if updates.upToDate}<span class="text-fg-muted">{t("agent.upToDate")}</span>{/if}
          {/if}
        </dd>
      </dl>
    {/if}

    <form onsubmit={change} class="flex flex-col gap-2 border-t border-border pt-4">
      <label class="label !mb-0" for="agent-port">{t("agent.port")}</label>
      <div class="flex gap-2">
        <input id="agent-port" class="input w-32 tabular-nums" bind:value={port} inputmode="numeric" maxlength="5" />
        <button type="submit" class="btn btn-primary" disabled={busy || !valid || Number(port) === currentPort}>
          {t(busy ? "agent.changing" : "agent.changePort")}
        </button>
      </div>
      {#if conn?.env === "dev"}
        <p class="hint">{t("agent.devHint")}</p>
      {:else}
        <p class="hint">{t("agent.prodHint")}</p>
      {/if}
      <p class="hint">{t("agent.terminal")} <span class="selectable font-mono">keeprun-agent set-port {valid ? port : "7272"}</span></p>
      {#if error}<p class="selectable alert" role="alert">{error}</p>{/if}
      {#if updates.error && !updates.installing}<p class="selectable alert" role="alert">{updates.error}</p>{/if}
      {#if message}<p class="selectable whitespace-pre-line text-[13px] text-fg-muted">{message}</p>{/if}
    </form>
  </div>
</dialog>
