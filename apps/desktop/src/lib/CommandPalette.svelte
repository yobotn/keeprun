<script lang="ts" module>
  export interface Command {
    id: string;
    label: string;
    group: string;
    hint?: string;
    run: () => void;
  }
</script>

<script lang="ts">
  import { Search } from "@lucide/svelte";

  let { commands, onclose }: { commands: Command[]; onclose: () => void } = $props();

  let dialog: HTMLDialogElement;
  let query = $state("");
  let active = $state(0);
  let list: HTMLUListElement | undefined = $state();

  $effect(() => {
    dialog.showModal();
  });

  // Every query word must appear in the label or group, in any order.
  const results = $derived.by(() => {
    const words = query.toLowerCase().split(/\s+/).filter(Boolean);
    return commands.filter((c) => {
      const hay = `${c.label} ${c.group}`.toLowerCase();
      return words.every((w) => hay.includes(w));
    });
  });

  $effect(() => {
    query;
    active = 0;
  });

  function run(c: Command | undefined) {
    if (!c) return;
    dialog.close();
    c.run();
  }

  function onkeydown(e: KeyboardEvent) {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      active = Math.min(results.length - 1, active + 1);
    } else if (e.key === "ArrowUp") {
      e.preventDefault();
      active = Math.max(0, active - 1);
    } else if (e.key === "Enter") {
      e.preventDefault();
      run(results[active]);
    } else return;
    queueMicrotask(() => list?.querySelector(`[data-i="${active}"]`)?.scrollIntoView({ block: "nearest" }));
  }
</script>

<dialog
  bind:this={dialog}
  {onclose}
  class="dialog mt-[12vh] w-[560px] max-w-[calc(100vw-32px)]"
  onclick={(e) => e.target === dialog && dialog.close()}
>
  <div class="flex items-center gap-2 border-b border-border px-4">
    <Search size={16} class="text-fg-muted" />
    <!-- svelte-ignore a11y_autofocus -->
    <input
      autofocus
      bind:value={query}
      {onkeydown}
      placeholder="Search apps and actions"
      class="h-12 flex-1 bg-transparent text-[15px] outline-none placeholder:text-fg-muted focus-visible:outline-none"
      role="combobox"
      aria-expanded="true"
      aria-controls="palette-list"
      aria-activedescendant="palette-{active}"
    />
    <span class="kbd">Esc</span>
  </div>
  <ul bind:this={list} id="palette-list" role="listbox" class="max-h-[360px] overflow-auto p-1.5">
    {#each results as c, i (c.id)}
      {#if i === 0 || results[i - 1].group !== c.group}
        <li class="px-2.5 pb-1 pt-2 text-[11px] font-semibold uppercase tracking-wider text-fg-muted" role="presentation">
          {c.group}
        </li>
      {/if}
      <li
        id="palette-{i}"
        data-i={i}
        role="option"
        aria-selected={i === active}
        class="flex items-center rounded-md px-2.5 py-2 text-[13px] {i === active ? 'bg-bg-active' : ''}"
        onmousemove={() => (active = i)}
        onclick={() => run(c)}
        onkeydown={() => {}}
      >
        <span class="flex-1">{c.label}</span>
        {#if c.hint}<span class="kbd">{c.hint}</span>{/if}
      </li>
    {:else}
      <li class="px-3 py-6 text-center text-sm text-fg-muted">No matches.</li>
    {/each}
  </ul>
</dialog>
