<script lang="ts">
  import { ArrowDown, Copy, Eraser, Pause, Play, Search } from "@lucide/svelte";
  import { api, config, formatTime, type LogLine } from "./api";
  import { locale, t } from "./i18n/index.svelte";

  // `taskSince` shows only task output from that time on (used by the Tasks tab).
  let { appId, taskSince }: { appId: string; taskSince?: number } = $props();

  // `seq` keys the rows, so dropping old lines does not redraw every row.
  type Row = LogLine & { seq: number };
  let lines = $state<Row[]>([]);
  let paused = $state(false);
  let query = $state("");
  let errorsOnly = $state(false);
  let connected = $state(false);
  let atBottom = $state(true);
  let copied = $state(false);
  let scroller: HTMLDivElement | undefined = $state();
  let searchInput: HTMLInputElement | undefined = $state();
  let pending: Row[] = [];
  let seq = 0;
  /** Lines held while paused or scrolled up, shown on the Latest button. */
  let held = $state(0);
  /** After a reconnect the agent replays its buffer: replace the lines then, instead of
   * clearing them on connect and leaving the pane blank until the replay arrives. */
  let replay = false;
  /** Shown as offline only after a moment, so a quick reconnect does not flash. */
  let offline = $state(false);

  // Scrolled up, new lines wait: the text being read stays put until you scroll down.
  const holding = $derived(paused || !atBottom);

  const visible = $derived.by(() => {
    const q = query.toLowerCase();
    return lines.filter(
      (l) =>
        (taskSince === undefined || (l.task && l.ts >= taskSince)) &&
        (!errorsOnly || l.stream === "stderr") &&
        (!q || l.text.toLowerCase().includes(q)),
    );
  });

  function flush() {
    if (holding || pending.length === 0) return;
    const next = replay ? pending : lines.concat(pending);
    replay = false;
    pending = [];
    held = 0;
    lines = next.length > config.logMaxLines ? next.slice(next.length - config.logMaxLines) : next;
  }

  $effect(() => {
    const id = appId;
    lines = [];
    pending = [];
    let ws: WebSocket | null = null;
    let closed = false;
    held = 0;
    replay = false;
    let retry: ReturnType<typeof setTimeout>;
    let offlineTimer: ReturnType<typeof setTimeout> | undefined;
    let emptyReplay: ReturnType<typeof setTimeout> | undefined;
    let frame = 0;

    const open = async () => {
      ws = await api.logSocket(id);
      ws.onopen = () => {
        connected = true;
        clearTimeout(offlineTimer);
        offline = false;
        // The agent replays its buffer on connect, which repeats anything still pending.
        pending = [];
        replay = true;
        // Nothing replayed, e.g. the agent restarted: the old lines are gone.
        emptyReplay = setTimeout(() => {
          if (replay && pending.length === 0) {
            replay = false;
            lines = [];
          }
        }, 1000);
      };
      ws.onmessage = (e) => {
        pending.push({ ...JSON.parse(e.data), seq: seq++ });
        if (pending.length > config.logMaxLines) pending.splice(0, pending.length - config.logMaxLines);
        // Batch per animation frame so busy apps do not freeze the UI.
        if (!frame)
          frame = requestAnimationFrame(() => {
            frame = 0;
            flush();
            held = pending.length;
          });
      };
      ws.onclose = () => {
        if (connected) offlineTimer = setTimeout(() => (offline = true), 3000);
        connected = false;
        if (!closed) retry = setTimeout(open, 2000);
      };
    };
    open();

    return () => {
      closed = true;
      clearTimeout(retry);
      clearTimeout(offlineTimer);
      clearTimeout(emptyReplay);
      if (frame) cancelAnimationFrame(frame);
      ws?.close();
    };
  });

  $effect(() => {
    if (!holding) flush();
  });

  $effect(() => {
    visible.length;
    if (scroller && atBottom) queueMicrotask(() => scroller && (scroller.scrollTop = scroller.scrollHeight));
  });

  function onScroll() {
    if (!scroller) return;
    atBottom = scroller.scrollHeight - scroller.scrollTop - scroller.clientHeight < 24;
  }

  function toBottom() {
    atBottom = true;
    if (scroller) scroller.scrollTop = scroller.scrollHeight;
  }

  function copy() {
    navigator.clipboard.writeText(visible.map((l) => l.text).join("\n"));
    copied = true;
    setTimeout(() => (copied = false), 1200);
  }

  function onkeydown(e: KeyboardEvent) {
    const typing = (e.target as HTMLElement)?.closest("input,textarea,select");
    if (e.key === "/" && !typing) {
      e.preventDefault();
      searchInput?.focus();
    } else if (e.key === "Escape" && e.target === searchInput) {
      query = "";
      searchInput?.blur();
    } else if (e.ctrlKey && e.key.toLowerCase() === "f") {
      e.preventDefault();
      searchInput?.focus();
      searchInput?.select();
    } else if (!typing && !e.ctrlKey && !e.altKey) {
      if (e.key === "p") paused = !paused;
      else if (e.key === "e") errorsOnly = !errorsOnly;
      else if (e.key === "G") toBottom();
    }
  }
</script>

<svelte:window {onkeydown} />

<div class="flex min-h-0 flex-1 flex-col">
  <div class="flex items-center gap-1.5 border-b border-border px-6 py-2">
    <label class="flex h-8 flex-1 items-center gap-2 rounded-md bg-bg-subtle px-2.5">
      <Search size={14} class="text-fg-muted" />
      <input
        bind:this={searchInput}
        bind:value={query}
        placeholder={t(taskSince === undefined ? "logs.search" : "logs.searchTask")}
        class="w-full bg-transparent text-[13px] outline-none placeholder:text-fg-muted"
      />
      {#if query}<span class="shrink-0 text-xs tabular-nums text-fg-muted">{t("logs.lines", { count: visible.length })}</span>{:else}<span class="kbd">/</span>{/if}
    </label>
    <button
      class="btn !h-8 {errorsOnly ? '!border-danger !text-danger' : ''}"
      onclick={() => (errorsOnly = !errorsOnly)}
      title={t("logs.errorsHint")}>{t("logs.errors")}</button
    >
    <button class="btn-icon" onclick={() => (paused = !paused)} title={t(paused ? "logs.resume" : "logs.pause")}>
      {#if paused}<Play size={15} class="flip-rtl" />{:else}<Pause size={15} />{/if}
    </button>
    <button class="btn-icon" onclick={copy} title={t("logs.copy")}><Copy size={15} /></button>
    <button
      class="btn-icon"
      onclick={() => {
        lines = [];
        pending = [];
        held = 0;
      }}
      title={t("logs.clear")}><Eraser size={15} /></button>
    <span class="min-w-20 text-end text-xs {offline ? 'text-warn' : 'text-fg-muted'}">
      {t(copied ? "common.copied" : paused ? "logs.paused" : offline || (!connected && lines.length === 0) ? "logs.connecting" : "logs.live")}
    </span>
  </div>
  <div class="relative min-h-0 flex-1">
    <div
      bind:this={scroller}
      onscroll={onScroll}
      class="selectable absolute inset-0 overflow-auto bg-log-bg px-6 py-2 font-mono text-[12.5px] leading-5"
      role="log"
      dir="ltr"
      aria-live="off"
    >
      {#if visible.length === 0}
        <p class="text-fg-muted" dir={locale.lang.rtl ? "rtl" : "ltr"}>
          {t(query || errorsOnly ? "logs.noMatch" : taskSince === undefined ? "logs.noOutput" : "logs.noTaskOutput")}
        </p>
      {/if}
      {#each visible as l (l.seq)}
        <div class="flex gap-3">
          <span class="shrink-0 tabular-nums text-fg-muted/70">{formatTime(l.ts)}</span>
          {#if l.task && taskSince === undefined}
            <span class="shrink-0 select-none text-accent" title={t("logs.taskBadgeHint")}>{t("logs.taskBadge")}</span>
          {/if}
          <span
            class="min-w-0 whitespace-pre-wrap break-all {l.stream === 'stderr'
              ? 'text-danger'
              : l.stream === 'system'
                ? 'text-fg-muted'
                : ''}">{l.text}</span
          >
        </div>
      {/each}
    </div>
    {#if !atBottom}
      <button class="btn absolute bottom-4 end-6 shadow-md" onclick={toBottom}><ArrowDown size={14} /> {t("logs.latest")}{#if held}<span class="tabular-nums text-fg-muted">+{held}</span>{/if}</button>
    {/if}
  </div>
</div>
