<script lang="ts">
  import { api, formatTime, type AppEvent, type EventKind } from "./api";

  let { appId, limit = 100, compact = false }: { appId: string; limit?: number; compact?: boolean } = $props();

  let events = $state<AppEvent[]>([]);
  let error = $state("");

  $effect(() => {
    const id = appId;
    let alive = true;
    const load = () =>
      api
        .events(id, limit)
        .then((e) => {
          if (alive) {
            events = e;
            error = "";
          }
        })
        .catch((e) => alive && (error = (e as Error).message));
    load();
    const t = setInterval(load, 2000);
    return () => {
      alive = false;
      clearInterval(t);
    };
  });

  const tone: Partial<Record<EventKind, string>> = {
    crashed: "text-danger",
    failed: "text-danger",
    "start-failed": "text-danger",
    restarted: "text-warn",
    started: "text-accent",
  };
</script>

{#if error}
  <p class="alert" role="alert">{error}</p>
{:else if events.length === 0}
  <p class="text-sm text-fg-muted">No events yet.</p>
{:else}
  <ol class="selectable flex flex-col">
    {#each events as e (e.id)}
      <li class="flex gap-4 py-1.5 text-[13px] {compact ? '' : 'border-b border-border last:border-0'}">
        <time class="w-28 shrink-0 tabular-nums text-fg-muted">{formatTime(e.ts, true)}</time>
        <span class={tone[e.kind] ?? ""}>{e.message}</span>
      </li>
    {/each}
  </ol>
{/if}
