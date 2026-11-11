<script lang="ts">
  import { X } from "@lucide/svelte";
  import { formatBytes } from "./api";
  import { installUpdate, updates } from "./updates.svelte";

  let { onclose }: { onclose: () => void } = $props();
  let dialog: HTMLDialogElement;

  $effect(() => {
    dialog.showModal();
  });

  const info = $derived(updates.available);
  const percent = $derived(updates.total ? Math.min(100, Math.round((updates.downloaded / updates.total) * 100)) : null);
  const date = $derived(info?.date ? new Date(info.date).toLocaleDateString([], { year: "numeric", month: "short", day: "numeric" }) : "");
</script>

<dialog bind:this={dialog} {onclose} class="dialog w-[520px] max-w-[calc(100vw-32px)]">
  <div class="flex flex-col gap-4 p-5">
    <div class="flex items-center justify-between">
      <h2 class="text-base font-semibold">Update Keeprun</h2>
      <button type="button" class="btn-icon" onclick={() => dialog.close()} title="Close (Esc)" disabled={updates.installing}>
        <X size={16} />
      </button>
    </div>

    {#if info}
      <p class="text-[13px]">
        Version <span class="font-semibold">{info.version}</span> is available. You have {info.currentVersion}.{date
          ? ` Released ${date}.`
          : ""}
      </p>
      {#if info.notes}
        <div class="selectable max-h-60 overflow-auto whitespace-pre-wrap rounded-md border border-border bg-bg-subtle px-3 py-2 text-[13px]">
          {info.notes}
        </div>
      {/if}
      <p class="hint !mt-0">
        Windows asks for administrator approval. The installer updates the desktop app, the agent service and the CLI,
        then restarts the service. Apps that were running start again.
      </p>
    {:else if !updates.error}
      <p class="text-[13px] text-fg-muted">No update available.</p>
    {/if}

    {#if updates.phase}
      <div class="flex flex-col gap-1.5">
        <div class="h-1.5 overflow-hidden rounded-full bg-bg-subtle">
          <div
            class="h-full rounded-full bg-accent transition-[width]"
            style="width: {updates.phase === 'installing' ? 100 : (percent ?? 30)}%"
          ></div>
        </div>
        <p class="text-xs text-fg-muted">
          {#if updates.phase === "installing"}
            Starting the installer. Keeprun closes while it runs.
          {:else}
            Downloading {formatBytes(updates.downloaded)}{updates.total ? ` of ${formatBytes(updates.total)}` : ""}
          {/if}
        </p>
      </div>
    {/if}

    {#if updates.error}<p class="selectable alert" role="alert">{updates.error}</p>{/if}

    <div class="flex justify-end gap-2">
      <button type="button" class="btn" onclick={() => dialog.close()} disabled={updates.installing}>Later</button>
      <button type="button" class="btn btn-primary" onclick={installUpdate} disabled={!info || updates.installing}>
        {updates.installing ? "Updating" : "Install and restart"}
      </button>
    </div>
  </div>
</dialog>
