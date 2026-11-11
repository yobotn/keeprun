<script lang="ts">
  import { X } from "@lucide/svelte";
  import { connection, desktop, type Connection } from "./api";
  import { checkForUpdates, updates } from "./updates.svelte";

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
      <h2 class="text-base font-semibold">Agent</h2>
      <button type="button" class="btn-icon" onclick={() => dialog.close()} title="Close (Esc)"><X size={16} /></button>
    </div>

    {#if conn}
      <dl class="selectable grid grid-cols-[110px_1fr] gap-y-1.5 text-[13px]">
        <dt class="text-fg-muted">Environment</dt>
        <dd>{conn.env}</dd>
        <dt class="text-fg-muted">Address</dt>
        <dd class="font-mono">{conn.baseUrl}</dd>
        <dt class="text-fg-muted">Data folder</dt>
        <dd class="truncate font-mono" title={conn.dataDir}>{conn.dataDir}</dd>
        <dt class="text-fg-muted">Version</dt>
        <dd class="flex items-center gap-3">
          {conn.version}
          {#if updates.available}
            <button type="button" class="text-accent hover:underline" onclick={onupdate}>Update to {updates.available.version}</button>
          {:else if updates.policy && !updates.policy.enabled}
            <span class="text-fg-muted">{updates.policy.reason}</span>
          {:else}
            <button type="button" class="text-accent hover:underline disabled:opacity-60" disabled={updates.checking} onclick={() => checkForUpdates(true)}>
              {updates.checking ? "Checking" : "Check for updates"}
            </button>
            {#if updates.upToDate}<span class="text-fg-muted">Up to date</span>{/if}
          {/if}
        </dd>
      </dl>
    {/if}

    <form onsubmit={change} class="flex flex-col gap-2 border-t border-border pt-4">
      <label class="label !mb-0" for="agent-port">Port</label>
      <div class="flex gap-2">
        <input id="agent-port" class="input w-32 tabular-nums" bind:value={port} inputmode="numeric" maxlength="5" />
        <button type="submit" class="btn btn-primary" disabled={busy || !valid || Number(port) === currentPort}>
          {busy ? "Changing" : "Change port"}
        </button>
      </div>
      {#if conn?.env === "dev"}
        <p class="hint">Saved to config/dev.local.toml. Restart the dev agent to use it.</p>
      {:else}
        <p class="hint">
          Windows asks for administrator approval. The agent restarts on the new port, and apps that were running
          start again. The CLI follows automatically.
        </p>
      {/if}
      <p class="hint">From a terminal: <span class="selectable font-mono">keeprun-agent set-port {valid ? port : "7272"}</span></p>
      {#if error}<p class="selectable alert" role="alert">{error}</p>{/if}
      {#if updates.error && !updates.installing}<p class="selectable alert" role="alert">{updates.error}</p>{/if}
      {#if message}<p class="selectable whitespace-pre-line text-[13px] text-fg-muted">{message}</p>{/if}
    </form>
  </div>
</dialog>
